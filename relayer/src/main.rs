use actix_web::dev::ServerHandle;
use anyhow::{anyhow, Context, Result as AnyResult};
use clap::Parser;
use eth_events_electra_client::traits::EthereumEventClient;
use ethereum_beacon_client::BeaconClient;
use ethereum_client::{EthApi, PollingEthApi};
use ethereum_common::SLOTS_PER_EPOCH;
use gclient::ext::sp_runtime::AccountId32;
use gear_common::api_provider::{ApiProvider, ApiProviderConnection};
use historical_proxy_client::{traits::HistoricalProxy as _, HistoricalProxy};
use primitive_types::U256;
use relayer::{
    cli::{
        BeaconRpcArgs, Cli, CliCommands, EthGearManualArgs, EthGearTokensArgs,
        EthGearTokensCommands, EthereumArgs, EthereumKillSwitchArgs, EthereumSignerArgs, FeePayers,
        FetchMerkleRootsArgs, GearEthTokensCommands, GearSignerArgs, DEFAULT_COUNT_CONFIRMATIONS,
    },
    ethereum_checkpoints, hex_utils,
    kill_switch::KillSwitchRelayer,
    message_relayer::{self, eth_to_gear, gear_to_eth},
    server,
};
use sails_rs::{calls::Query, gclient::calls::GClientRemoting, ActorId};
use std::{
    collections::HashSet, fs::File, io::Read as _, net::TcpListener, path::Path, str::FromStr,
    time::Duration,
};

use tokio::{sync::mpsc, task};
use utils_prometheus::MetricsBuilder;
use vft_manager_client::traits::VftManager;
use zeroize::Zeroizing;

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
        .filter(Some("relayer"), log::LevelFilter::Info)
        .filter(Some("ethereum-client"), log::LevelFilter::Info)
        .filter(Some("metrics"), log::LevelFilter::Info)
        .format_timestamp_secs()
        .parse_default_env()
        .init();

    let cli = Cli::parse();

    match cli.command {
        CliCommands::KillSwitch(args) => {
            let api_provider = ApiProvider::new(
                args.gear_args.get_endpoint()?,
                args.gear_args.max_reconnect_attempts,
            )
            .await
            .expect("Failed to connect to Gear API");

            let eth_observer_api = create_eth_killswitch_client(&args.ethereum_args)
                .await
                .expect("Failed to create Ethereum client");

            let metrics = MetricsBuilder::new();

            let mut kill_switch = KillSwitchRelayer::new(
                api_provider.connection(),
                eth_observer_api,
                args.from_eth_block,
            )
            .await;

            metrics
                .register_service(&kill_switch)
                .build()
                .run(args.prometheus_args.prometheus_endpoint)
                .await;
            api_provider.spawn();
            kill_switch.run().await.expect("Kill switch relayer failed");
        }

        CliCommands::QueueCleaner(args) => {
            let api_provider = ApiProvider::new(
                args.gear_args.get_endpoint()?,
                args.gear_args.max_reconnect_attempts,
            )
            .await?;
            let conn = api_provider.connection();
            api_provider.spawn();

            relayer::queue_cleaner::queue_cleaner(conn, args.suri, args.delay).await?;
        }

        CliCommands::GearEthTokens(args) => {
            let eth_api = create_eth_signer_client(&args.ethereum_args).await;

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_endpoint: args.gear_args.get_endpoint()?,
            };

            let provider = ApiProvider::new(
                gsdk_args.vara_endpoint.clone(),
                args.gear_args.max_reconnect_attempts,
            )
            .await
            .context("Failed to create API provider")?;

            let api = provider.connection().client();
            let governance_admin: [u8; 32] = AccountId32::from_str(&args.governance_admin)
                .expect("Failed to parse governance admin address")
                .into();
            let governance_admin: ActorId = ActorId::from(governance_admin);
            let governance_pauser: [u8; 32] = AccountId32::from_str(&args.governance_pauser)
                .expect("Failed to parse governance pauser address")
                .into();
            let governance_pauser: ActorId = ActorId::from(governance_pauser);

            let mut excluded_from_fees = HashSet::new();
            match args.no_fee {
                None => {
                    log::warn!("No free from charge accounts listed, using default: bridgeAdmin and bridgePauser from chain constants");
                    match api.bridge_admin().await {
                        Ok(admin) => {
                            log::info!("Bridge admin: {admin}");
                            let admin: &[u8] = admin.as_ref();
                            excluded_from_fees.insert(AccountId32::try_from(admin).unwrap());
                        }
                        Err(e) => {
                            log::error!("Failed to get bridge admin: {e}");
                        }
                    };

                    match api.bridge_pauser().await {
                        Ok(pauser) => {
                            log::info!("Bridge pauser: {pauser}");
                            let pauser: &[u8] = pauser.as_ref();
                            excluded_from_fees.insert(AccountId32::try_from(pauser).unwrap());
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
                        args.from_block,
                        args.confirmations_merkle_root
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                        args.confirmations_status
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                        args.storage_path,
                        (governance_admin, governance_pauser),
                    )
                    .await
                    .unwrap();

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(args.prometheus_args.prometheus_endpoint)
                        .await;

                    provider.spawn();
                    relayer.run().await?;
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
                    let web_server = server::create(
                        tcp_listener,
                        web_server_token,
                        "gear-eth-token-paid-transfers".to_string(),
                        Some(sender),
                        None,
                    )
                    .context("Failed to create web server")?;
                    let handle_server = web_server.handle();
                    task::spawn(web_server);

                    let relayer = gear_to_eth::paid_token_transfers::Relayer::new(
                        eth_api,
                        bridging_payment_address,
                        provider.connection(),
                        args.from_block,
                        args.confirmations_merkle_root
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                        args.confirmations_status
                            .unwrap_or(DEFAULT_COUNT_CONFIRMATIONS),
                        excluded_from_fees,
                        receiver,
                        args.storage_path.clone(),
                        governance_admin,
                        governance_pauser,
                    )
                    .await
                    .unwrap();

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(args.prometheus_args.prometheus_endpoint)
                        .await;

                    provider.spawn();
                    let result = relayer.run().await;
                    stop_web_server(handle_server).await;
                    result?;
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
                .run(args.prometheus_args.prometheus_endpoint)
                .await;

            relayer.run().await?;
        }
        CliCommands::EthGearTokens(EthGearTokensArgs {
            command,
            vft_manager_address,
            gear_args,
            ethereum_rpc,
            beacon_rpc,
            prometheus_args,
            storage_path,
            ethereum_blocks,
            ethereum_start_block,
            cutover_seal,
            successor_manager_code_id,
        }) => {
            let eth_api = PollingEthApi::new(&ethereum_rpc).await?;
            let approved_manager_code = successor_manager_code_id
                .as_deref()
                .map(hex_utils::decode_h256)
                .transpose()?;
            let beacon_client = create_beacon_client(&beacon_rpc).await;

            let gsdk_args = message_relayer::common::GSdkArgs {
                vara_endpoint: gear_args.connection.get_endpoint()?,
            };
            let provider = ApiProvider::new(
                gsdk_args.vara_endpoint.clone(),
                gear_args.connection.max_reconnect_attempts,
            )
            .await
            .expect("Failed to create API provider");
            let connection = provider.connection();
            provider.spawn();

            let vft_manager_address =
                hex_utils::decode_h256(&vft_manager_address).expect("Failed to parse address");

            let (historical_proxy_address, checkpoint_light_client_address) =
                fetch_historical_proxy_and_checkpoints(
                    connection.clone(),
                    vft_manager_address.0.into(),
                    &gear_args.suri,
                )
                .await
                .expect("Failed to fetch historical proxy");

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
                        checkpoint_light_client_address.into_bytes().into(),
                        historical_proxy_address.into_bytes().into(),
                        vft_manager_address,
                        connection,
                        storage_path,
                        genesis_time,
                        ethereum_start_block,
                        ethereum_blocks.clone(),
                        cutover_seal.clone(),
                        approved_manager_code,
                    )
                    .await
                    .expect("Failed to create relayer");

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(prometheus_args.prometheus_endpoint)
                        .await;

                    relayer.run().await?;
                }
                EthGearTokensCommands::PaidTokenTransfers {
                    bridging_payment_address,
                    web_server_token,
                    web_server_address,
                } => {
                    let bridging_payment_address =
                        hex_utils::decode_h160(&bridging_payment_address)
                            .expect("Failed to parse address");

                    let tcp_listener = TcpListener::bind(web_server_address)?;
                    let (sender, receiver) = mpsc::unbounded_channel();
                    let web_server = server::create(
                        tcp_listener,
                        web_server_token,
                        "eth-gear-token-paid-transfers".to_string(),
                        None,
                        Some(sender),
                    )
                    .context("Failed to create web server")?;
                    let handle_server = web_server.handle();
                    task::spawn(web_server);

                    let relayer = eth_to_gear::paid_token_transfers::Relayer::new(
                        gear_args.suri,
                        eth_api,
                        beacon_client,
                        bridging_payment_address,
                        checkpoint_light_client_address.into_bytes().into(),
                        historical_proxy_address.into_bytes().into(),
                        vft_manager_address,
                        connection,
                        storage_path,
                        genesis_time,
                        ethereum_start_block,
                        ethereum_blocks.clone(),
                        Some(receiver),
                        cutover_seal.clone(),
                        approved_manager_code,
                    )
                    .await
                    .expect("Failed to create relayer");

                    MetricsBuilder::new()
                        .register_service(&relayer)
                        .build()
                        .run(prometheus_args.prometheus_endpoint)
                        .await;

                    let result = relayer.run().await;
                    stop_web_server(handle_server).await;
                    result?;
                }
            }
        }
        CliCommands::GearEthManual(args) => {
            let nonce =
                hex_utils::decode_byte_vec(&args.nonce).expect("Failed to parse message nonce");
            let nonce = U256::from_big_endian(&nonce[..]);
            let eth_api = create_eth_signer_client(&args.ethereum_args).await;
            let api_provider = ApiProvider::new(
                args.gear_args.get_endpoint()?,
                args.gear_args.max_reconnect_attempts,
            )
            .await
            .expect("Failed to create API provider");

            let governance_admin: [u8; 32] = AccountId32::from_str(&args.governance_admin)
                .expect("Failed to parse governance admin address")
                .into();
            let governance_admin: ActorId = ActorId::from(governance_admin);
            let governance_pauser: [u8; 32] = AccountId32::from_str(&args.governance_pauser)
                .expect("Failed to parse governance pauser address")
                .into();
            let governance_pauser: ActorId = ActorId::from(governance_pauser);

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
                governance_admin,
                governance_pauser,
                &args.storage,
            )
            .await?;
        }

        CliCommands::EthGearManual(EthGearManualArgs {
            tx_hash,
            storage,
            cutover_seal,
            successor_manager_code_id,
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
                vara_endpoint: gear_args.connection.get_endpoint()?,
            };
            let eth_api = PollingEthApi::new(&ethereum_rpc).await?;
            let beacon_client = create_beacon_client(&beacon_args).await;
            let provider = ApiProvider::new(
                gear_client_args.vara_endpoint.clone(),
                gear_args.connection.max_reconnect_attempts,
            )
            .await
            .context("Failed to create API provider")?;

            let provider_connection = provider.connection();

            provider.spawn();

            let receiver_address = hex_utils::decode_h256(&receiver_program)
                .expect("Failed to decode receiver program address");

            let (historical_proxy_address, checkpoint_light_client_address) = if receiver_route
                .is_some()
            {
                let historical_proxy_address = historical_proxy
                    .as_ref()
                    .map(|address| {
                        hex_utils::decode_h256(address)
                            .expect("Failed to parse historical proxy address")
                    })
                    .ok_or_else(|| {
                        anyhow!(
                            "historical-proxy argument is required if receiver-route is specified"
                        )
                    })?;
                let checkpoint_light_client_address = checkpoint_light_client
                    .as_ref()
                    .map(|address| {
                        hex_utils::decode_h256(address)
                            .expect("Failed to parse checkpoint light client address")
                    })
                    .ok_or_else(|| anyhow!("checkpoint-light-client argument is required if receiver-route is specified"))?;
                (
                    historical_proxy_address.0.into(),
                    checkpoint_light_client_address.0.into(),
                )
            } else {
                fetch_historical_proxy_and_checkpoints(
                    provider_connection.clone(),
                    receiver_address.0.into(),
                    &gear_args.suri,
                )
                .await?
            };

            let tx_hash = hex_utils::decode_h256(&tx_hash)
                .expect("Failed to decode tx hash")
                .0
                .into();

            let receiver_route = receiver_route
                .map(|receiver_route| {
                    hex_utils::decode_byte_vec(&receiver_route)
                        .expect("Failed to decode receiver route")
                })
                .unwrap_or(vft_manager_client::vft_manager::io::SubmitReceipt::ROUTE.to_vec());

            eth_to_gear::manual::relay(
                provider_connection,
                gear_args.suri,
                eth_api,
                beacon_client,
                checkpoint_light_client_address.into_bytes().into(),
                historical_proxy_address.into_bytes().into(),
                receiver_address,
                receiver_route,
                tx_hash,
                &storage,
                cutover_seal.as_deref(),
                successor_manager_code_id
                    .as_deref()
                    .map(hex_utils::decode_h256)
                    .transpose()?,
            )
            .await?;
        }

        CliCommands::FetchMerkleRoots(args) => fetch_merkle_roots(args).await?,
    };

    Ok(())
}

async fn stop_web_server(handle: ServerHandle) {
    if tokio::time::timeout(Duration::from_secs(5 * 60), handle.stop(true))
        .await
        .is_err()
    {
        log::error!("Failed to stop web server within timeout");
        std::process::exit(1);
    }
}

async fn create_gclient_client(args: &GearSignerArgs) -> gclient::GearApi {
    let endpoint = args.connection.get_endpoint().expect("Invalid gear args");
    gclient::GearApi::builder()
        .suri(&args.suri)
        .uri(endpoint)
        .build()
        .await
        .expect("GearApi client should be created")
}

async fn create_eth_signer_client(args: &EthereumSignerArgs) -> EthApi {
    let EthereumArgs {
        connection,
        tx,
        mq_address,
        queue_deployment_transaction,
    } = &args.ethereum_args;

    EthApi::new_with_retries(
        &connection.ethereum_endpoint,
        mq_address,
        Some(&args.eth_fee_payer),
        connection.max_retries,
        connection.retry_interval_ms.map(Duration::from_millis),
        tx.max_fee_per_gas,
        tx.max_priority_fee_per_gas,
    )
    .await
    .expect("Error while creating ethereum client")
    .with_queue_deployment_transaction(*queue_deployment_transaction)
}

async fn create_eth_killswitch_client(args: &EthereumKillSwitchArgs) -> AnyResult<EthApi> {
    fn read_pk_bytes<P: AsRef<Path>>(path: P, buf: &mut Zeroizing<[u8; 32]>) -> AnyResult<()> {
        let path = path.as_ref();
        let mut file = File::open(path)?;
        if file.read(buf.as_mut())? != 32 {
            return Err(anyhow!("Invalid key length in file: {path:?}"));
        }

        Ok(())
    }

    let mut buf = Zeroizing::<[u8; 32]>::default();

    read_pk_bytes(&args.eth_observer_pk_path, &mut buf).with_context(|| {
        format!(
            "Failed to read ETH_OBSERVER_PK_PATH file: {:?}",
            &args.eth_observer_pk_path
        )
    })?;
    let observer_pk_str = format!("0x{}", hex::encode(buf.as_ref()));

    let observer_api = create_eth_signer_client(&EthereumSignerArgs {
        ethereum_args: args.ethereum_args.clone(),
        eth_fee_payer: observer_pk_str,
    })
    .await;

    Ok(observer_api)
}

async fn create_eth_client(args: &EthereumArgs) -> EthApi {
    EthApi::new(
        &args.connection.ethereum_endpoint,
        &args.mq_address,
        None,
        args.tx.max_fee_per_gas,
        args.tx.max_priority_fee_per_gas,
    )
    .await
    .expect("Error while creating ethereum client")
    .with_queue_deployment_transaction(args.queue_deployment_transaction)
}

async fn create_beacon_client(args: &BeaconRpcArgs) -> BeaconClient {
    let timeout = args.timeout.map(Duration::from_secs);

    BeaconClient::new(args.beacon_endpoint.clone(), timeout)
        .await
        .expect("Failed to create beacon client")
}

async fn fetch_merkle_roots(args: FetchMerkleRootsArgs) -> AnyResult<()> {
    let eth_api = create_eth_client(&args.ethereum_args).await;
    let mut view = eth_api.verified_finalized_view().await?;
    let block_finalized = view.block_number();

    if args.from_eth_block > block_finalized {
        return Ok(());
    }

    let start = ethereum_client::get_block(eth_api.raw_provider(), args.from_eth_block).await?;
    anyhow::ensure!(
        start.header.number == args.from_eth_block,
        "HOLD: root scan start header height changed"
    );
    let merkle_roots = eth_api
        .fetch_merkle_roots_in_range(args.from_eth_block, block_finalized)
        .await?;
    let mut targets = std::collections::BTreeMap::from([
        (args.from_eth_block, start.header.hash),
        (view.block_number(), view.block_hash()),
    ]);
    for observation in &merkle_roots {
        anyhow::ensure!(
            !observation.removed,
            "HOLD: removed merkle root observation"
        );
        if let Some(old) = targets.insert(observation.block_number, observation.block_hash) {
            anyhow::ensure!(
                old == observation.block_hash,
                "HOLD: conflicting root inclusion hashes"
            );
        }
    }
    view.verify_blocks(eth_api.raw_provider(), &targets).await?;

    let gear_api = gear_rpc_client::GearApi::new(
        &args.gear_args.get_endpoint()?,
        args.gear_args.max_reconnect_attempts,
    )
    .await?;

    for observation in merkle_roots {
        let source_block = u32::try_from(observation.entry.block_number)?;
        anyhow::ensure!(
            eth_api
                .read_merkle_root_in_view(source_block, &view)
                .await?
                == Some(observation.entry.merkle_root.0),
            "HOLD: observed merkle root differs from finalized queue state"
        );
        let timestamp = eth_api
            .get_block_timestamp_by_hash(observation.block_number, observation.block_hash)
            .await?;
        let block_hash = gear_api.block_number_to_hash(source_block).await?;
        let authority_set_id = gear_api.signed_by_authority_set_id(block_hash).await?;
        log::info!("{observation:?}, block_hash = {block_hash:?}, authority_set_id = {authority_set_id}, timestamp = {timestamp}");
    }

    Ok(())
}

async fn fetch_historical_proxy_and_checkpoints(
    mut api_provider: ApiProviderConnection,
    vft_manager_address: ActorId,
    suri: &str,
) -> anyhow::Result<(ActorId, ActorId)> {
    log::info!("Fetching historical proxy address and checkpoint light client address from VFT Manager at {vft_manager_address:#?}");
    let client = api_provider.gclient_client(suri)?;
    let remoting = GClientRemoting::new(client);
    let vft_manager = vft_manager_client::VftManager::new(remoting.clone());
    let historical_proxy = HistoricalProxy::new(remoting.clone());
    let eth_events = eth_events_electra_client::EthereumEventClient::new(remoting);

    let historical_proxy_address = vft_manager
        .historical_proxy_address()
        .recv(vft_manager_address)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to receive historical proxy address: {e:?}"))?;
    log::info!("Historical proxy address is {historical_proxy_address:#?}");
    let endpoints = historical_proxy
        .endpoints()
        .recv(historical_proxy_address)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to receive endpoints: {e:?}"))?;
    let (slot, endpoint) = endpoints
        .into_iter()
        .max_by_key(|(slot, _)| *slot)
        .ok_or_else(|| anyhow::anyhow!("No endpoints found in historical proxy"))?;

    let checkpoints_address = eth_events
        .checkpoint_light_client_address()
        .recv(endpoint)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to receive endpoint: {e:?}"))?;

    log::info!("Checkpoint light client address is {checkpoints_address:#?} for slot #{slot}");

    Ok((historical_proxy_address, checkpoints_address))
}
