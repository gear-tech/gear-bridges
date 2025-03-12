use std::time::Duration;

use clap::Parser;

use ethereum_beacon_client::BeaconClient;
use ethereum_client::EthApi;
use gear_rpc_client::GearApi;
use kill_switch::KillSwitchRelayer;
use message_relayer::{eth_to_gear, gear_to_eth};
use proof_storage::{FileSystemProofStorage, GearProofStorage, ProofStorage};
use prover::proving::GenesisConfig;
use relay_merkle_roots::MerkleRootRelayer;
use utils_prometheus::MetricsBuilder;

mod cli;
mod common;
mod ethereum_checkpoints;
mod hex_utils;
mod kill_switch;
mod message_relayer;
mod proof_storage;
mod prover_interface;
mod relay_merkle_roots;

use cli::{
    BeaconRpcArgs, Cli, CliCommands, EthGearManualArgs, EthGearTokensArgs, EthGearTokensCommands,
    EthereumArgs, EthereumSignerArgs, GearArgs, GearEthTokensCommands, GearSignerArgs,
    GenesisConfigArgs, ProofStorageArgs,
};

#[tokio::main]
async fn main() {
    let _ = dotenv::dotenv();

    pretty_env_logger::formatted_timed_builder()
        .filter_level(log::LevelFilter::Off)
        .format_target(false)
        .filter(Some("prover"), log::LevelFilter::Info)
        .filter(Some("relayer"), log::LevelFilter::Info)
        .filter(Some("ethereum-client"), log::LevelFilter::Info)
        .filter(Some("metrics"), log::LevelFilter::Info)
        .format_timestamp_secs()
        .parse_default_env()
        .init();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::GearEthCore(args) => {
            let gear_api = create_gear_client(&args.gear_args).await;
            let eth_api = create_eth_signer_client(&args.ethereum_args);

            let metrics = MetricsBuilder::new();

            let (proof_storage, metrics) =
                create_proof_storage(&args.proof_storage_args, &args.gear_args, metrics).await;

            let genesis_config = create_genesis_config(&args.genesis_config_args);

            let relayer =
                MerkleRootRelayer::new(gear_api, eth_api, genesis_config, proof_storage).await;

            metrics
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            relayer.run().await.expect("Merkle root relayer failed");
        }
        CliCommands::KillSwitch(args) => {
            let gear_api = create_gear_client(&args.gear_args).await;
            let eth_api = create_eth_signer_client(&args.ethereum_args);

            let metrics = MetricsBuilder::new();

            let (proof_storage, metrics) =
                create_proof_storage(&args.proof_storage_args, &args.gear_args, metrics).await;

            let genesis_config = create_genesis_config(&args.genesis_config_args);

            let block_finality_storage =
                sled::open("./block_finality_storage").expect("Db not corrupted");

            let mut kill_switch = KillSwitchRelayer::new(
                gear_api,
                eth_api,
                genesis_config,
                proof_storage,
                args.from_eth_block,
                block_finality_storage,
            )
            .await;

            metrics
                .register_service(&kill_switch)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            kill_switch.run().await.expect("Kill switch relayer failed");
        }
        CliCommands::GearEthTokens(args) => {
            let gear_api = create_gear_client(&args.gear_args).await;
            let eth_api = create_eth_signer_client(&args.ethereum_args);

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_domain: args.gear_args.domain,
                vara_port: args.gear_args.port,
                vara_rpc_retries: args.gear_args.retries,
            };

            let mut metrics_builder = MetricsBuilder::new();

            match args.command {
                GearEthTokensCommands::AllTokenTransfers => {
                    let relayer = gear_to_eth::all_token_transfers::Relayer::new(
                        gear_api,
                        gsdk_args,
                        eth_api,
                        args.from_block,
                    )
                    .await
                    .unwrap();

                    metrics_builder = metrics_builder.register_service(&relayer);

                    relayer.run();
                }
                GearEthTokensCommands::PaidTokenTransfers {
                    bridging_payment_address,
                } => {
                    let bridging_payment_address =
                        hex_utils::decode_h256(&bridging_payment_address)
                            .expect("Failed to parse address");

                    let relayer = gear_to_eth::paid_token_transfers::Relayer::new(
                        gear_api,
                        gsdk_args,
                        eth_api,
                        args.from_block,
                        bridging_payment_address,
                    )
                    .await
                    .unwrap();

                    metrics_builder = metrics_builder.register_service(&relayer);

                    relayer.run();
                }
            }

            metrics_builder
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            loop {
                // relayer.run() spawns thread and exits, so we need to add this loop after calling run.
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        CliCommands::EthGearCore(args) => {
            let gear_api = create_gclient_client(&args.gear_args).await;

            let beacon_client = create_beacon_client(&args.beacon_args).await;

            let program_id =
                hex_utils::decode_h256(&args.program_id).expect("Failed to decode program_id");
            let multiplier = (args.size_batch_multiplier > 0)
                .then(|| args.size_batch_multiplier)
                .unwrap_or(1);
            let relayer = ethereum_checkpoints::Relayer::new(
                program_id,
                beacon_client,
                gear_api,
                args.size_batch.get().wrapping_mul(multiplier),
            );

            MetricsBuilder::new()
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            relayer.run().await;
        }
        CliCommands::EthGearTokens(EthGearTokensArgs {
            command,
            checkpoint_light_client_address,
            historical_proxy_address,
            vft_manager_address,
            gear_args,
            ethereum_args,
            beacon_rpc,
            prometheus_args,
        }) => {
            let eth_api = create_eth_client(&ethereum_args);
            let beacon_client = create_beacon_client(&beacon_rpc).await;

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_domain: gear_args.common.domain,
                vara_port: gear_args.common.port,
                vara_rpc_retries: gear_args.common.retries,
            };

            let checkpoint_light_client_address =
                hex_utils::decode_h256(&checkpoint_light_client_address)
                    .expect("Failed to parse address");
            let historical_proxy_address =
                hex_utils::decode_h256(&historical_proxy_address).expect("Failed to parse address");
            let vft_manager_address =
                hex_utils::decode_h256(&vft_manager_address).expect("Failed to parse address");

            match command {
                EthGearTokensCommands::AllTokenTransfers {
                    erc20_manager_address,
                } => {
                    let erc20_manager_address = hex_utils::decode_h160(&erc20_manager_address)
                        .expect("Failed to parse address");

                    let relayer = eth_to_gear::all_token_transfers::Relayer::new(
                        gsdk_args,
                        gear_args.suri,
                        eth_api,
                        beacon_client,
                        erc20_manager_address,
                        checkpoint_light_client_address,
                        historical_proxy_address,
                        vft_manager_address,
                    )
                    .await
                    .expect("Failed to create relayer");

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(prometheus_args.endpoint)
                        .await;

                    relayer.run();
                }
                EthGearTokensCommands::PaidTokenTransfers {
                    bridging_payment_address,
                } => {
                    let bridging_payment_address =
                        hex_utils::decode_h160(&bridging_payment_address)
                            .expect("Failed to parse address");

                    let relayer = eth_to_gear::paid_token_transfers::Relayer::new(
                        gsdk_args,
                        gear_args.suri,
                        eth_api,
                        beacon_client,
                        bridging_payment_address,
                        checkpoint_light_client_address,
                        historical_proxy_address,
                        vft_manager_address,
                    )
                    .await
                    .expect("Failed to create relayer");

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(prometheus_args.endpoint)
                        .await;

                    relayer.run();
                }
            }

            loop {
                // relayer.run() spawns thread and exits, so we need to add this loop after calling run.
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        CliCommands::GearEthManual(args) => {
            let nonce = hex_utils::decode_h256(&args.nonce).expect("Failed to parse message nonce");
            let eth_api = create_eth_signer_client(&args.ethereum_args);
            let gear_api = create_gear_client(&args.gear_args).await;

            gear_to_eth::manual::relay(gear_api, eth_api, nonce, args.block, args.from_eth_block)
                .await;

            loop {
                // relay() spawns thread and exits, so we need to add this loop after calling run.
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
        CliCommands::EthGearManual(EthGearManualArgs {
            tx_hash,
            slot,
            checkpoint_light_client,
            historical_proxy,
            receiver_program,
            receiver_route,
            gear_args,
            ethereum_args,
            beacon_args,
        }) => {
            let gear_client_args = message_relayer::common::GSdkArgs {
                vara_domain: gear_args.common.domain,
                vara_port: gear_args.common.port,
                vara_rpc_retries: gear_args.common.retries,
            };
            let eth_api = create_eth_client(&ethereum_args);
            let beacon_client = create_beacon_client(&beacon_args).await;
            let checkpoint_light_client_address = hex_utils::decode_h256(&checkpoint_light_client)
                .expect("Failed to parse checkpoint light client address");
            let historical_proxy_address = hex_utils::decode_h256(&historical_proxy)
                .expect("Failed to parse historical proxy address");
            let receiver_address = hex_utils::decode_h256(&receiver_program)
                .expect("Failed to parse receiver program address");
            let receiver_route = hex_utils::decode_byte_vec(&receiver_route)
                .expect("Failed to decode receiver route");
            let tx_hash = hex_utils::decode_h256(&tx_hash)
                .expect("Failed to decode tx hash")
                .0
                .into();

            eth_to_gear::manual::relay(
                gear_client_args,
                gear_args.suri,
                eth_api,
                beacon_client,
                checkpoint_light_client_address,
                historical_proxy_address,
                receiver_address,
                receiver_route,
                tx_hash,
                slot,
            )
            .await;

            loop {
                // relay() spawns thread and exits, so we need to add this loop after calling run.
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    };
}

async fn create_gclient_client(args: &GearSignerArgs) -> gclient::GearApi {
    gclient::GearApi::builder()
        .retries(args.common.retries)
        .suri(&args.suri)
        .build(gclient::WSAddress::new(
            &args.common.domain,
            args.common.port,
        ))
        .await
        .expect("GearApi client should be created")
}

async fn create_gear_client(args: &GearArgs) -> GearApi {
    GearApi::new(&args.domain, args.port, args.retries)
        .await
        .unwrap_or_else(|err| panic!("Error while creating gear client: {}", err))
}

fn create_eth_signer_client(args: &EthereumSignerArgs) -> EthApi {
    let EthereumArgs {
        eth_endpoint,
        relayer_address,
        mq_address,
        ..
    } = &args.ethereum_args;

    EthApi::new(
        eth_endpoint,
        mq_address,
        relayer_address,
        Some(&args.fee_payer),
    )
    .expect("Error while creating ethereum client")
}

fn create_eth_client(args: &EthereumArgs) -> EthApi {
    let EthereumArgs {
        eth_endpoint,
        relayer_address,
        mq_address,
        ..
    } = args;

    EthApi::new(eth_endpoint, mq_address, relayer_address, None)
        .expect("Error while creating ethereum client")
}

async fn create_beacon_client(args: &BeaconRpcArgs) -> BeaconClient {
    let timeout = args.beacon_timeout.map(Duration::from_secs);

    BeaconClient::new(args.beacon_endpoint.clone(), timeout)
        .await
        .expect("Failed to create beacon client")
}

async fn create_proof_storage(
    proof_storage_args: &ProofStorageArgs,
    gear_args: &GearArgs,
    mut metrics: MetricsBuilder,
) -> (Box<dyn ProofStorage>, MetricsBuilder) {
    let proof_storage: Box<dyn ProofStorage> =
        if let Some(fee_payer) = proof_storage_args.gear_fee_payer.as_ref() {
            let proof_storage = GearProofStorage::new(
                &gear_args.domain,
                gear_args.port,
                gear_args.retries,
                fee_payer,
                "./onchain_proof_storage_data".into(),
            )
            .await
            .expect("Failed to initialize proof storage");

            metrics = metrics.register_service(&proof_storage);

            Box::from(proof_storage)
        } else {
            log::warn!("Fee payer not present, falling back to FileSystemProofStorage");
            Box::from(FileSystemProofStorage::new("./proof_storage".into()))
        };

    (proof_storage, metrics)
}

fn create_genesis_config(genesis_config_args: &GenesisConfigArgs) -> GenesisConfig {
    let authority_set_hash = hex::decode(&genesis_config_args.authority_set_hash)
        .expect("Incorrect format for authority set hash: hex-encoded hash is expected");
    let authority_set_hash = authority_set_hash
        .try_into()
        .expect("Incorrect format for authority set hash: wrong length");

    GenesisConfig {
        authority_set_id: genesis_config_args.authority_set_id,
        authority_set_hash,
    }
}
