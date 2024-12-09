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
    BeaconRpcArgs, Cli, CliCommands, EthereumArgs, GenesisConfigArgs, ProofStorageArgs,
    RelayErc20Args, RelayErc20Commands, VaraArgs,
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
        CliCommands::RelayMerkleRoots(args) => {
            let gear_api = create_gear_client(&args.vara_args).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            let metrics = MetricsBuilder::new();

            let (proof_storage, metrics) =
                create_proof_storage(&args.proof_storage_args, &args.vara_args, metrics).await;

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
            let gear_api = create_gear_client(&args.vara_args).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            let metrics = MetricsBuilder::new();

            let (proof_storage, metrics) =
                create_proof_storage(&args.proof_storage_args, &args.vara_args, metrics).await;

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
        CliCommands::RelayMessages(args) => {
            let gear_api = create_gear_client(&args.vara_args).await;
            let eth_api = create_eth_client(&args.ethereum_args);

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_domain: args.vara_args.vara_domain,
                vara_port: args.vara_args.vara_port,
                vara_rpc_retries: args.vara_args.vara_rpc_retries,
            };
            if let Some(bridging_payment_address) = args.bridging_payment_address {
                let bridging_payment_address = hex_utils::decode_h256(&bridging_payment_address)
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

                MetricsBuilder::new()
                    .register_service(&relayer)
                    .build()
                    .run(args.prometheus_args.endpoint)
                    .await;

                relayer.run();
            } else {
                let relayer = gear_to_eth::all_token_transfers::Relayer::new(
                    gear_api,
                    gsdk_args,
                    eth_api,
                    args.from_block,
                )
                .await
                .unwrap();

                MetricsBuilder::new()
                    .register_service(&relayer)
                    .build()
                    .run(args.prometheus_args.endpoint)
                    .await;

                relayer.run();
            }

            loop {
                // relayer.run() spawns thread and exits, so we need to add this loop after calling run.
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        CliCommands::RelayCheckpoints(args) => {
            let gear_api = create_gclient_client(&args.vara_args, args.vara_suri).await;

            let beacon_client = create_beacon_client(&args.beacon_args).await;

            let program_id =
                hex_utils::decode_h256(&args.program_id).expect("Failed to decode program_id");

            let relayer = ethereum_checkpoints::Relayer::new(program_id, beacon_client, gear_api);

            MetricsBuilder::new()
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;

            relayer.run().await;
        }
        CliCommands::RelayErc20(RelayErc20Args {
            command,
            checkpoint_light_client_address,
            historical_proxy_address,
            vft_manager_address,
            vara_args,
            vara_suri,
            ethereum_args,
            beacon_rpc,
            prometheus_args,
        }) => {
            let eth_api = create_eth_client(&ethereum_args);
            let beacon_client = create_beacon_client(&beacon_rpc).await;

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_domain: vara_args.vara_domain,
                vara_port: vara_args.vara_port,
                vara_rpc_retries: vara_args.vara_rpc_retries,
            };

            let checkpoint_light_client_address =
                hex_utils::decode_h256(&checkpoint_light_client_address)
                    .expect("Failed to parse address");
            let historical_proxy_address =
                hex_utils::decode_h256(&historical_proxy_address).expect("Failed to parse address");
            let vft_manager_address =
                hex_utils::decode_h256(&vft_manager_address).expect("Failed to parse address");

            match command {
                RelayErc20Commands::AllTokenTransfers {
                    erc20_treasury_address,
                } => {
                    let erc20_treasury_address = hex_utils::decode_h160(&erc20_treasury_address)
                        .expect("Failed to parse address");

                    let relayer = eth_to_gear::all_token_transfers::Relayer::new(
                        gsdk_args,
                        vara_suri,
                        eth_api,
                        beacon_client,
                        erc20_treasury_address,
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
                RelayErc20Commands::PaidTokenTransfers {
                    bridging_payment_address,
                } => {
                    let bridging_payment_address =
                        hex_utils::decode_h160(&bridging_payment_address)
                            .expect("Failed to parse address");

                    let relayer = eth_to_gear::paid_token_transfers::Relayer::new(
                        gsdk_args,
                        vara_suri,
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
    };
}

async fn create_gclient_client(args: &VaraArgs, suri: String) -> gclient::GearApi {
    gclient::GearApi::builder()
        .retries(args.vara_rpc_retries)
        .suri(suri)
        .build(gclient::WSAddress::new(&args.vara_domain, args.vara_port))
        .await
        .expect("GearApi client should be created")
}

async fn create_gear_client(args: &VaraArgs) -> GearApi {
    GearApi::new(&args.vara_domain, args.vara_port, args.vara_rpc_retries)
        .await
        .unwrap_or_else(|err| panic!("Error while creating gear client: {}", err))
}

fn create_eth_client(args: &EthereumArgs) -> EthApi {
    let EthereumArgs {
        eth_endpoint,
        fee_payer,
        relayer_address,
        mq_address,
    } = args;

    EthApi::new(
        eth_endpoint,
        mq_address,
        relayer_address,
        fee_payer.as_deref(),
    )
    .unwrap_or_else(|err| panic!("Error while creating ethereum client: {}", err))
}

async fn create_beacon_client(args: &BeaconRpcArgs) -> BeaconClient {
    let timeout = args.beacon_timeout.map(Duration::from_secs);

    BeaconClient::new(args.beacon_endpoint.clone(), timeout)
        .await
        .expect("Failed to create beacon client")
}

async fn create_proof_storage(
    proof_storage_args: &ProofStorageArgs,
    vara_args: &VaraArgs,
    mut metrics: MetricsBuilder,
) -> (Box<dyn ProofStorage>, MetricsBuilder) {
    let proof_storage: Box<dyn ProofStorage> =
        if let Some(fee_payer) = proof_storage_args.gear_fee_payer.as_ref() {
            let proof_storage = GearProofStorage::new(
                &vara_args.vara_domain,
                vara_args.vara_port,
                vara_args.vara_rpc_retries,
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
