use super::{
    cutover::{self, JournalOwnership},
    message_sender::MessageSender,
    proof_composer::ProofComposer,
    storage::{InboundRuntimeIdentity, JSONStorage, Storage},
    tx_manager::TransactionManager,
};
use crate::message_relayer::common::{
    ethereum::{
        block_listener::BlockListener as EthereumBlockListener, block_storage::JSONBlockStorage,
        deposit_event_extractor::DepositEventExtractor,
    },
    gear::{
        block_listener::BlockListener as GearBlockListener, block_storage::NoStorage,
        checkpoints_extractor::CheckpointsExtractor,
    },
    EthereumSlotNumber,
};
use ethereum_beacon_client::BeaconClient;
use ethereum_client::PollingEthApi;
use gear_common::api_provider::ApiProviderConnection;
use primitive_types::{H160, H256};
use sails_rs::calls::ActionIo;
use std::{iter, sync::Arc};
use utils_prometheus::MeteredService;

pub struct Relayer {
    gear_block_listener: GearBlockListener,
    ethereum_block_listener: EthereumBlockListener,

    deposit_event_extractor: DepositEventExtractor,
    checkpoints_extractor: CheckpointsExtractor,
    latest_checkpoint: Option<EthereumSlotNumber>,

    proof_composer: ProofComposer,
    gear_message_sender: MessageSender,

    storage: Arc<dyn Storage>,

    tx_manager: TransactionManager,
    _journal_ownership: Arc<JournalOwnership>,
}

impl MeteredService for Relayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        iter::empty()
            .chain(self.gear_block_listener.get_sources())
            .chain(self.ethereum_block_listener.get_sources())
            .chain(self.deposit_event_extractor.get_sources())
            .chain(self.checkpoints_extractor.get_sources())
    }
}

impl Relayer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        suri: String,
        eth_api: PollingEthApi,
        beacon_client: BeaconClient,
        erc20_manager_address: H160,
        checkpoint_light_client_address: H256,
        historical_proxy_address: H256,
        vft_manager_address: H256,
        mut api_provider: ApiProviderConnection,
        storage_path: String,
        genesis_time: u64,
        from_eth_block: u64,
        eth_unprocessed_block_storage_path: String,
        cutover_seal: Option<std::path::PathBuf>,
        approved_manager_code: Option<H256>,
    ) -> anyhow::Result<Self> {
        cutover::reject_sealed(std::path::Path::new(&storage_path))?;
        cutover::reject_sealed(std::path::Path::new(&eth_unprocessed_block_storage_path))?;
        let journal_ownership = Arc::new(cutover::lock_original_journals(
            std::path::Path::new(&storage_path),
            std::path::Path::new(&eth_unprocessed_block_storage_path),
        )?);
        let gear_block_listener = GearBlockListener::new(api_provider.clone(), Arc::new(NoStorage));

        anyhow::ensure!(
            std::path::Path::new(&storage_path).join("state.json").try_exists()?
                == std::path::Path::new(&eth_unprocessed_block_storage_path).try_exists()?,
            "HOLD: inbound transaction/discovery journal pair is incomplete; reconcile the original deployment coverage"
        );

        let storage = Arc::new(
            JSONStorage::new(&storage_path).with_journal_ownership(journal_ownership.clone()),
        );

        let checkpoints_extractor = CheckpointsExtractor::new(checkpoint_light_client_address);

        let client = api_provider
            .gclient_client(&suri)
            .expect("failed to construct gclient");

        let gear_genesis_hash = crate::rpc::retry_gear(
            &mut api_provider,
            "inbound Gear genesis",
            |api| async move { api.block_number_to_hash(0).await },
        )
        .await?;
        let identity = InboundRuntimeIdentity {
            ethereum_chain_id: eth_api.chain_id().await?,
            ethereum_genesis_hash: eth_api.get_block(0).await?.header.hash.0.into(),
            ethereum_start_block: from_eth_block,
            erc20_manager_address: Some(erc20_manager_address),
            bridging_payment_address: None,
            gear_genesis_hash,
            vft_manager_address,
            checkpoint_light_client_address,
            historical_proxy_address,
            gear_sender: client.account_id().clone().into(),
        };
        let signer_ownership =
            cutover::lock_signer(identity.gear_genesis_hash, identity.gear_sender)?;
        eth_api
            .enable_finality_archive(
                &std::path::Path::new(&storage_path).join("ethereum-finality"),
                alloy::primitives::B256::from(identity.ethereum_genesis_hash.0),
            )
            .await?;
        let predecessor = cutover::authenticate_startup(
            &identity,
            cutover_seal.as_deref(),
            approved_manager_code,
            &client,
            &eth_api,
        )
        .await?;
        log::info!(
            "Authenticated inbound runtime: {}; predecessor_cutover: {}",
            serde_json::to_string(&identity)?,
            serde_json::to_string(&predecessor)?
        );
        let (minimum_receipt_slot, last_receipt_slot) =
            cutover::receipt_domain(&client, vft_manager_address).await?;
        log::info!("Authenticated inbound receipt domain: minimum={minimum_receipt_slot}; last={last_receipt_slot:?}; independently approved successor CodeId={approved_manager_code:?}");
        let block_storage = Arc::new(
            JSONBlockStorage::new_bound(
                eth_unprocessed_block_storage_path.into(),
                identity.clone(),
                predecessor.clone(),
            )
            .await?
            .with_journal_ownership(journal_ownership.clone()),
        );
        if let Some(predecessor) = predecessor {
            storage
                .bind_successor_identity(identity, predecessor)
                .await?;
        } else {
            storage.bind_runtime_identity(identity).await?;
        }
        let ethereum_block_listener =
            EthereumBlockListener::new(eth_api.clone(), block_storage.clone())
                .with_last_receipt_slot(genesis_time, last_receipt_slot);
        let deposit_event_extractor = DepositEventExtractor::new(
            eth_api.clone(),
            erc20_manager_address,
            storage.clone(),
            genesis_time,
            block_storage,
        );

        let latest_checkpoint =
            super::get_latest_checkpoint(checkpoint_light_client_address, client).await;

        let route =
            <vft_manager_client::vft_manager::io::SubmitReceipt as ActionIo>::ROUTE.to_vec();

        let gear_message_sender = MessageSender::new(
            vft_manager_address,
            route,
            historical_proxy_address,
            api_provider.clone(),
            suri.clone(),
            Some(erc20_manager_address),
        )
        .with_signer_ownership(signer_ownership);

        let proof_composer = ProofComposer::new(
            api_provider,
            beacon_client,
            eth_api,
            historical_proxy_address,
            suri,
        );

        let tx_manager = TransactionManager::new(storage.clone());

        Ok(Self {
            _journal_ownership: journal_ownership,
            gear_block_listener,
            ethereum_block_listener,

            deposit_event_extractor,
            checkpoints_extractor,
            latest_checkpoint,

            proof_composer,
            gear_message_sender,

            storage,
            tx_manager,
        })
    }

    pub async fn run(self) -> anyhow::Result<()> {
        self.storage.load(&self.tx_manager).await?;

        let [gear_blocks] = self.gear_block_listener.run().await;
        let ethereum_blocks = self.ethereum_block_listener.spawn();

        let deposit_events = self.deposit_event_extractor.run(ethereum_blocks).await;

        let checkpoints = self
            .checkpoints_extractor
            .run(gear_blocks, self.latest_checkpoint)
            .await;
        let proof_composer = self.proof_composer.run(checkpoints);
        let message_sender = self.gear_message_sender.run();

        self.tx_manager
            .run(deposit_events, proof_composer, message_sender)
            .await
    }
}
