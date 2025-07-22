use crate::cli::FeePayers;
use anyhow::{anyhow, Context, Result as AnyResult};
use clap::Parser;
use cli::{
    BeaconRpcArgs, Cli, CliCommands, EthGearManualArgs, EthGearTokensArgs, EthGearTokensCommands,
    EthereumArgs, EthereumSignerArgs, FetchMerkleRootsArgs, GearArgs, GearEthTokensCommands,
    GearSignerArgs, GenesisConfigArgs, ProofStorageArgs, DEFAULT_COUNT_CONFIRMATIONS,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, PollingEthApi};
use ethereum_common::SLOTS_PER_EPOCH;
use gclient::ext::sp_runtime::AccountId32;
use kill_switch::KillSwitchRelayer;
use message_relayer::{
    eth_to_gear::{self, api_provider::ApiProvider},
    gear_to_eth,
};
use primitive_types::U256;
use proof_storage::{FileSystemProofStorage, GearProofStorage, ProofStorage};
use prover::proving::GenesisConfig;
use relay_merkle_roots::MerkleRootRelayer;
use relayer::*;
use std::{collections::HashSet, net::TcpListener, str::FromStr, time::Duration};
use tokio::{sync::mpsc, task, time};
use utils_prometheus::MetricsBuilder;

fn main() -> AnyResult<()> {
    // we need at least 2 native threads to run some of the blocking tasks like proof composition
    // so lets set minimum to 4 threads or to available parallelism.
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(std::thread::available_parallelism()?.get().max(4))
        .build()?
        .block_on(run())
}

async fn run() -> AnyResult<()> {
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
            let api_provider = ApiProvider::new(
                args.gear_args.domain.clone(),
                args.gear_args.port,
                args.gear_args.retries,
            )
            .await
            .expect("Failed to connect to Gear API");
            let eth_api = create_eth_signer_client(&args.ethereum_args).await;

            let metrics = MetricsBuilder::new();

            let (proof_storage, metrics) =
                create_proof_storage(&args.proof_storage_args, &args.gear_args, metrics).await;

            let genesis_config = create_genesis_config(&args.genesis_config_args);

            let block_listener = message_relayer::common::gear::block_listener::BlockListener::new(
                api_provider.connection(),
            );

            let relayer = MerkleRootRelayer::new(
                api_provider.connection(),
                eth_api,
                genesis_config,
                proof_storage,
                args.start_authority_set_id,
                args.confirmations_merkle_root
                    .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
            )
            .await;

            metrics
                .register_service(&relayer)
                .build()
                .run(args.prometheus_args.endpoint)
                .await;
            api_provider.spawn();

            let [blocks] = block_listener.run().await;
            relayer
                .run(blocks)
                .await
                .expect("Merkle root relayer failed");
        }
        CliCommands::KillSwitch(args) => {
            let api_provider = ApiProvider::new(
                args.gear_args.domain.clone(),
                args.gear_args.port,
                args.gear_args.retries,
            )
            .await
            .expect("Failed to connec to Gear API");

            let eth_api = create_eth_signer_client(&args.ethereum_args).await;

            let metrics = MetricsBuilder::new();

            let (proof_storage, metrics) =
                create_proof_storage(&args.proof_storage_args, &args.gear_args, metrics).await;

            let genesis_config = create_genesis_config(&args.genesis_config_args);

            let block_finality_storage =
                sled::open("./block_finality_storage").expect("Db not corrupted");

            let mut kill_switch = KillSwitchRelayer::new(
                api_provider.connection(),
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
            api_provider.spawn();
            kill_switch.run().await.expect("Kill switch relayer failed");
        }

        CliCommands::GearEthTokens(args) => {
            let eth_api = create_eth_signer_client(&args.ethereum_args).await;

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_domain: args.gear_args.domain,
                vara_port: args.gear_args.port,
                vara_rpc_retries: args.gear_args.retries,
            };

            let provider = ApiProvider::new(
                gsdk_args.vara_domain.clone(),
                gsdk_args.vara_port,
                gsdk_args.vara_rpc_retries,
            )
            .await
            .context("Failed to create API provider")?;

            let api = provider.connection().client();

            let mut excluded_from_fees = HashSet::new();
            match args.no_fee {
                None => {
                    log::warn!("No free from charge accounts listed, using default: bridgeAdmin and bridgePauser from chain constants");
                    match api.bridge_admin().await.map(AccountId32::from) {
                        Ok(admin) => {
                            log::info!("Bridge admin: {admin}");
                            excluded_from_fees.insert(admin);
                        }
                        Err(e) => {
                            log::error!("Failed to get bridge admin: {e}");
                        }
                    };

                    match api.bridge_pauser().await.map(AccountId32::from) {
                        Ok(pauser) => {
                            log::info!("Bridge pauser: {pauser}");
                            excluded_from_fees.insert(pauser);
                        }
                        Err(e) => {
                            log::error!("Failed to get bridge pauser: {e}");
                        }
                    };

                    if excluded_from_fees.is_empty() {
                        return Err(anyhow!("Exiting"));
                    }
                }

                Some(FeePayers::All) => {
                    log::info!("All accounts haave to pay fees");
                }

                Some(FeePayers::ExcludedIds(ids)) => {
                    for id in ids {
                        let account_id = AccountId32::from_str(id.as_str())
                            .map_err(|e| anyhow!(r#"Failed to decode address "{id}": {e:?}"#))?;

                        log::debug!("Account {account_id} is excluded from paying fees");
                        excluded_from_fees.insert(account_id);
                    }
                }
            }

            match args.command {
                GearEthTokensCommands::AllTokenTransfers => {
                    let relayer = gear_to_eth::all_token_transfers::Relayer::new(
                        eth_api,
                        provider.connection(),
                        args.confirmations_merkle_root
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                        args.confirmations_status
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                    )
                    .await
                    .unwrap();

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(args.prometheus_args.endpoint)
                        .await;

                    provider.spawn();
                    relayer.run().await;

                    loop {
                        // relayer.run() spawns thread and exits, so we need to add this loop after calling run.
                        time::sleep(Duration::from_secs(1)).await;
                    }
                }

                GearEthTokensCommands::PaidTokenTransfers {
                    bridging_payment_address,
                    web_server_token,
                    web_server_address,
                } => {
                    let bridging_payment_address =
                        hex_utils::decode_h256(&bridging_payment_address)
                            .context("Failed to parse address")?;

                    // spawn web-server
                    let tcp_listener = TcpListener::bind(web_server_address)?;
                    let (sender, receiver) = mpsc::unbounded_channel();
                    let web_server = server::create(tcp_listener, web_server_token, sender)
                        .context("Failed to create web server")?;
                    let handle_server = web_server.handle();
                    task::spawn(web_server);

                    let relayer = gear_to_eth::paid_token_transfers::Relayer::new(
                        eth_api,
                        bridging_payment_address,
                        provider.connection(),
                        args.confirmations_merkle_root
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                        args.confirmations_status
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                        excluded_from_fees,
                        receiver,
                    )
                    .await
                    .unwrap();

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(args.prometheus_args.endpoint)
                        .await;

                    provider.spawn();
                    relayer.run().await;

                    tokio::signal::ctrl_c().await?;

                    handle_server.stop(true).await;
                }
            }
        }
        CliCommands::EthGearCore(args) => {
            let gear_api = create_gclient_client(&args.gear_args).await;

            let beacon_client = create_beacon_client(&args.beacon_args).await;

            let program_id =
                hex_utils::decode_h256(&args.program_id).expect("Failed to decode program_id");
            let multiplier = if args.size_batch_multiplier > 0 {
                args.size_batch_multiplier
            } else {
                1
            };
            let relayer = ethereum_checkpoints::Relayer::new(
                program_id,
                beacon_client,
                gear_api,
                multiplier.saturating_mul(SLOTS_PER_EPOCH),
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
            ethereum_rpc,
            beacon_rpc,
            prometheus_args,
            storage_path,
        }) => {
            let eth_api = PollingEthApi::new(&ethereum_rpc).await?;
            let beacon_client = create_beacon_client(&beacon_rpc).await;

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_domain: gear_args.common.domain,
                vara_port: gear_args.common.port,
                vara_rpc_retries: gear_args.common.retries,
            };
            let provider = ApiProvider::new(
                gsdk_args.vara_domain.clone(),
                gsdk_args.vara_port,
                gsdk_args.vara_rpc_retries,
            )
            .await
            .expect("Failed to create API provider");
            let checkpoint_light_client_address =
                hex_utils::decode_h256(&checkpoint_light_client_address)
                    .expect("Failed to parse address");
            let historical_proxy_address =
                hex_utils::decode_h256(&historical_proxy_address).expect("Failed to parse address");
            let vft_manager_address =
                hex_utils::decode_h256(&vft_manager_address).expect("Failed to parse address");

            let genesis_time = beacon_client
                .get_genesis()
                .await
                .expect("failed to fetch chain genesis")
                .data
                .genesis_time;

            log::debug!("Genesis time: {genesis_time}");

            match command {
                EthGearTokensCommands::AllTokenTransfers {
                    erc20_manager_address,
                } => {
                    let erc20_manager_address = hex_utils::decode_h160(&erc20_manager_address)
                        .expect("Failed to parse address");

                    let relayer = eth_to_gear::all_token_transfers::Relayer::new(
                        gear_args.suri,
                        eth_api,
                        beacon_client,
                        erc20_manager_address,
                        checkpoint_light_client_address,
                        historical_proxy_address,
                        vft_manager_address,
                        provider.connection(),
                        storage_path,
                        genesis_time,
                    )
                    .await
                    .expect("Failed to create relayer");

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(prometheus_args.endpoint)
                        .await;

                    provider.spawn();
                    relayer.run().await;
                }
                EthGearTokensCommands::PaidTokenTransfers {
                    bridging_payment_address,
                } => {
                    let bridging_payment_address =
                        hex_utils::decode_h160(&bridging_payment_address)
                            .expect("Failed to parse address");

                    let relayer = eth_to_gear::paid_token_transfers::Relayer::new(
                        gear_args.suri,
                        eth_api,
                        beacon_client,
                        bridging_payment_address,
                        checkpoint_light_client_address,
                        historical_proxy_address,
                        vft_manager_address,
                        provider.connection(),
                        storage_path,
                        genesis_time,
                    )
                    .await
                    .expect("Failed to create relayer");

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(prometheus_args.endpoint)
                        .await;
                    provider.spawn();
                    relayer.run().await;
                }
            }
        }
        CliCommands::GearEthManual(args) => {
            let nonce =
                hex_utils::decode_byte_vec(&args.nonce).expect("Failed to parse message nonce");
            let nonce = U256::from_big_endian(&nonce[..]);
            let eth_api = create_eth_signer_client(&args.ethereum_args).await;
            let api_provider = ApiProvider::new(
                args.gear_args.domain.clone(),
                args.gear_args.port,
                args.gear_args.retries,
            )
            .await
            .expect("Failed to create API provider");

            let connection = api_provider.connection();
            api_provider.spawn();

            gear_to_eth::manual::relay(
                connection,
                eth_api,
                nonce,
                args.block,
                args.from_eth_block,
                args.confirmations_status
                    .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
            )
            .await;
        }

        CliCommands::EthGearManual(EthGearManualArgs {
            tx_hash,
            checkpoint_light_client,
            historical_proxy,
            receiver_program,
            receiver_route,
            gear_args,
            ethereum_rpc,
            beacon_args,
        }) => {
            use sails_rs::calls::ActionIo;

            let gear_client_args = message_relayer::common::GSdkArgs {
                vara_domain: gear_args.common.domain,
                vara_port: gear_args.common.port,
                vara_rpc_retries: gear_args.common.retries,
            };
            let eth_api = PollingEthApi::new(&ethereum_rpc).await?;
            let beacon_client = create_beacon_client(&beacon_args).await;

            let checkpoint_light_client_address = hex_utils::decode_h256(&checkpoint_light_client)
                .expect("Failed to parse checkpoint light client address");
            let historical_proxy_address = hex_utils::decode_h256(&historical_proxy)
                .expect("Failed to parse historical proxy address");
            let receiver_address = hex_utils::decode_h256(&receiver_program)
                .expect("Failed to parse receiver program address");
            let receiver_route = receiver_route
                .map(|receiver_route| {
                    hex_utils::decode_byte_vec(&receiver_route)
                        .expect("Failed to decode receiver route")
                })
                .unwrap_or(vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec());
            let tx_hash = hex_utils::decode_h256(&tx_hash)
                .expect("Failed to decode tx hash")
                .0
                .into();

            let provider = ApiProvider::new(
                gear_client_args.vara_domain.clone(),
                gear_client_args.vara_port,
                gear_client_args.vara_rpc_retries,
            )
            .await
            .context("Failed to create API provider")?;

            let provider_connection = provider.connection();

            provider.spawn();

            eth_to_gear::manual::relay(
                provider_connection,
                gear_args.suri,
                eth_api,
                beacon_client,
                checkpoint_light_client_address,
                historical_proxy_address,
                receiver_address,
                receiver_route,
                tx_hash,
            )
            .await?;

            loop {
                // relay() spawns thread and exits, so we need to add this loop after calling run.
                time::sleep(Duration::from_secs(1)).await;
            }
        }

        CliCommands::FetchMerkleRoots(args) => fetch_merkle_roots(args).await?,
    };

    Ok(())
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

async fn create_eth_signer_client(args: &EthereumSignerArgs) -> EthApi {
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
    .await
    .expect("Error while creating ethereum client")
}

async fn create_eth_client(args: &EthereumArgs) -> EthApi {
    let EthereumArgs {
        eth_endpoint,
        relayer_address,
        mq_address,
        ..
    } = args;

    EthApi::new(eth_endpoint, mq_address, relayer_address, None)
        .await
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

async fn fetch_merkle_roots(args: FetchMerkleRootsArgs) -> AnyResult<()> {
    let eth_api = create_eth_client(&args.ethereum_args).await;
    let block_finalized = eth_api.finalized_block_number().await?;

    if args.from_eth_block > block_finalized {
        return Ok(());
    }

    let block_range = common::create_range(args.from_eth_block.into(), block_finalized);
    let merkle_roots = eth_api
        .fetch_merkle_roots_in_range(block_range.from, block_range.to)
        .await?;

    let gear_api = gear_rpc_client::GearApi::new(
        &args.gear_args.domain,
        args.gear_args.port,
        args.gear_args.retries,
    )
    .await?;

    for (root, block_number_eth) in merkle_roots {
        let block_hash = gear_api
            .block_number_to_hash(root.block_number as u32)
            .await?;
        let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;

        log::info!("{root:?}, block_hash = {block_hash:?}, authority_set_id = {authority_set_id}, block_number_eth = {block_number_eth:?}");
    }

    Ok(())
}
