use std::{collections::BTreeMap, ops::AsyncFnMut, time::Duration};

use alloy::{
    primitives::{Address, U256},
    providers::Provider,
    rpc::types::{BlockId, Header},
};
use anyhow::Context;
use thiserror::Error;

use ethereum_client::{
    abi::IMessageQueue, EthApi, FinalizedView, MerkleRootEntry, MerkleRootObservation, TxHash,
    TxStatus,
};
use gear_common::api_provider::ApiProviderConnection;
use primitive_types::H256;
use prometheus::{Gauge, IntCounter, IntGauge};
use utils_prometheus::{impl_metered_service, MeteredService};

use crate::common::is_rpc_transport_error_recoverable;

const SCAN_EVENTS_PERIOD_SEC: Duration = Duration::from_secs(12);
const ERROR_REPEAT_DELAY: Duration = Duration::from_secs(3);

impl_metered_service! {
    struct Metrics {
        fee_payer_balance: Gauge = Gauge::new(
            "kill_switch_fee_payer_balance",
            "Transaction fee payer balance",
        ),
        latest_eth_block: IntGauge = IntGauge::new(
            "kill_switch_latest_eth_block",
            "Latest block number observed",
        ),
        merkle_roots_discovered_cnt: IntCounter = IntCounter::new(
            "kill_switch_merkle_roots_discovered_cnt",
            "Amount of merkle root events discovered",
        ),
        merkle_root_mismatch_cnt: IntCounter = IntCounter::new(
            "kill_switch_merkle_root_mismatch_cnt",
            "Amount of merkle root mismatches found",
        ),
        challenge_sent_cnt: IntCounter = IntCounter::new(
            "kill_switch_challenge_sent_cnt",
            "Amount of challenge sends finalized",
        ),
    }
}

#[derive(Debug, Error)]
enum ScanForEventsError {
    #[error("Ethereum API error: {0}")]
    EthApi(#[from] ethereum_client::Error),
    #[error("Gear API error: {0}")]
    GearApi(#[from] anyhow::Error),
    #[error("Other error: {0}")]
    Other(anyhow::Error),
}

#[derive(Debug, Error)]
enum Error {
    #[error("Scan for events error: {0}")]
    ScanForEvents(#[from] ScanForEventsError),
    #[error("Challenge root error: {0}")]
    ChallengeRoot(ethereum_client::Error),
}

#[derive(Debug, PartialEq, Eq)]
enum State {
    ScanForEvents,
    ChallengeRoot { tx_hash: Option<TxHash> },
    Exit,
}

impl State {
    fn complete_scan(
        &mut self,
        cursor: &mut Option<u64>,
        finalized_block: u64,
        result: Result<bool, ScanForEventsError>,
    ) -> Result<(), ScanForEventsError> {
        if result? {
            *self = Self::ChallengeRoot { tx_hash: None };
        } else {
            *cursor = Some(cursor.unwrap_or(0).max(finalized_block.saturating_add(1)));
        }
        Ok(())
    }

    fn record_submission(
        &mut self,
        result: Result<TxHash, ethereum_client::Error>,
    ) -> Result<(), ethereum_client::Error> {
        match result {
            Ok(tx_hash) => {
                *self = Self::ChallengeRoot {
                    tx_hash: Some(tx_hash),
                };
                Ok(())
            }
            Err(error) => {
                // Only typed, transient estimation failures prove no send was
                // attempted. Rescan first: a hot mismatching root may have reorged.
                *self = if matches!(&error, ethereum_client::Error::ChallengeRootPreBroadcast(_))
                    && ethereum_client::error::is_transient_rpc_cause(&error)
                {
                    Self::ScanForEvents
                } else {
                    // An ambiguous send requires operator reconciliation, never
                    // a blind second challenge without the original hash.
                    Self::Exit
                };
                Err(error)
            }
        }
    }

    fn record_receipt(
        &mut self,
        result: Result<TxStatus, ethereum_client::Error>,
    ) -> Result<bool, ethereum_client::Error> {
        match result? {
            TxStatus::Finalized => {
                *self = Self::Exit;
                Ok(true)
            }
            TxStatus::Pending => Ok(false),
            // A canonical finalized revert permits a fresh scan, not a retry
            // against a hot root that may since have been reorganized away.
            TxStatus::Failed => {
                *self = Self::ScanForEvents;
                Ok(false)
            }
        }
    }
}

pub struct KillSwitchRelayer {
    api_provider: ApiProviderConnection,
    eth_observer_api: EthApi,
    start_from_eth_block: Option<u64>,
    state: State,
    metrics: Metrics,
}

impl MeteredService for KillSwitchRelayer {
    fn get_sources(&self) -> impl IntoIterator<Item = Box<dyn prometheus::core::Collector>> {
        self.metrics.get_sources()
    }
}

impl KillSwitchRelayer {
    pub async fn new(
        api_provider: ApiProviderConnection,
        eth_observer_api: EthApi,
        from_eth_block: Option<u64>,
    ) -> Self {
        Self {
            api_provider,
            eth_observer_api,
            start_from_eth_block: from_eth_block,
            state: State::ScanForEvents,
            metrics: Metrics::new(),
        }
    }

    pub async fn run(&mut self) -> anyhow::Result<()> {
        log::info!("Starting kill switch relayer with observer role");
        loop {
            let res = match &self.state {
                State::ScanForEvents => self.scan_for_events().await.map_err(Error::from),
                State::ChallengeRoot { .. } => {
                    self.challenge_root().await.map_err(Error::ChallengeRoot)
                }
                State::Exit => {
                    anyhow::bail!(
                        "Kill switch stopped: separately authorized operator intervention required; \
                         no automatic root repair or unpause. The challenge pause is finite; \
                         persistent incidents require the governance pause"
                    );
                }
            };

            let loop_delay = match res {
                Ok(()) if self.state == State::ScanForEvents => SCAN_EVENTS_PERIOD_SEC,
                Ok(()) if self.state == (State::ChallengeRoot { tx_hash: None }) => Duration::ZERO,
                Ok(()) => Duration::from_secs(5),
                Err(err) => {
                    log::error!("Error in kill switch relayer: {err}");
                    match err {
                        Error::ScanForEvents(ScanForEventsError::EthApi(
                            ethereum_client::Error::ErrorInHTTPTransport(err),
                        ))
                        | Error::ChallengeRoot(ethereum_client::Error::ErrorInHTTPTransport(err))
                        | Error::ChallengeRoot(
                            ethereum_client::Error::ChallengeRootPreBroadcast(
                                alloy::contract::Error::TransportError(err),
                            ),
                        ) => {
                            if is_rpc_transport_error_recoverable(&err) {
                                match self.eth_observer_api.reconnect().await {
                                    Ok(reconnected) => self.eth_observer_api = reconnected,
                                    Err(e) => log::error!("Ethereum API reconnect failed: {e}"),
                                }
                            }
                        }
                        Error::ScanForEvents(ScanForEventsError::GearApi(_)) => {
                            if let Err(e) = self.api_provider.reconnect().await {
                                log::error!("Gear API reconnect failed: {e}");
                            }
                        }
                        _ => {}
                    }
                    ERROR_REPEAT_DELAY
                }
            };
            tokio::time::sleep(loop_delay).await;
        }
    }

    async fn scan_for_events(&mut self) -> Result<(), ScanForEventsError> {
        let balance = self.eth_observer_api.get_approx_balance().await?;
        self.metrics.fee_payer_balance.set(balance);
        let mut view = self.eth_observer_api.verified_finalized_view().await?;
        let finalized = view.block_number();
        // Retain the finalized cursor across failures and rescan the hot suffix
        // until it finalizes, including any replacement blocks after a reorg.
        let from = *self.start_from_eth_block.get_or_insert(finalized);
        let gear_api = self.api_provider.client();
        let mut source_finalized = None;
        let result = scan_roots(
            &self.eth_observer_api,
            &mut view,
            from,
            &self.metrics,
            async |entry: &MerkleRootEntry| {
                let finalized = match source_finalized {
                    Some(height) => height,
                    None => {
                        let hash = gear_api.latest_finalized_block().await?;
                        let height = gear_api.block_hash_to_number(hash).await?;
                        source_finalized = Some(height);
                        height
                    }
                };
                let source = source_height(entry, finalized)?;
                // The operator-configured source RPC remains the trust boundary.
                let hash = gear_api.block_number_to_hash(source).await?;
                let height = gear_api.block_hash_to_number(hash).await?;
                let (_, root) = gear_api.fetch_queue_merkle_root(hash).await?;
                compare_source_root(entry, finalized, Ok((height, root)))
            },
        )
        .await;
        self.state
            .complete_scan(&mut self.start_from_eth_block, finalized, result)
    }

    async fn challenge_root(&mut self) -> Result<(), ethereum_client::Error> {
        if let State::ChallengeRoot {
            tx_hash: Some(tx_hash),
        } = self.state
        {
            let status = self.eth_observer_api.get_tx_status(tx_hash).await;
            if self.state.record_receipt(status)? {
                self.metrics.challenge_sent_cnt.inc();
                log::warn!(
                    "Challenge {tx_hash:#x} finalized; exiting for separately authorized \
                     operator intervention, without root repair or unpause"
                );
            }
            return Ok(());
        }
        log::info!("Sending challenge root to Ethereum");
        let result = self.eth_observer_api.send_challenge_root().await;
        self.state.record_submission(result)
    }
}

async fn scan_roots(
    api: &EthApi,
    view: &mut FinalizedView,
    from: u64,
    metrics: &Metrics,
    mut compare_source: impl AsyncFnMut(&MerkleRootEntry) -> Result<bool, ScanForEventsError>,
) -> Result<bool, ScanForEventsError> {
    let head = ethereum_client::latest_block(api.raw_provider())
        .await
        .map_err(downcast_anyhow_to_ethereum_client)?
        .header;
    metrics
        .latest_eth_block
        .set(i64::try_from(head.number).unwrap_or(i64::MAX));
    if head.number < view.block_number() {
        return Err(ScanForEventsError::Other(anyhow::anyhow!(
            "HOLD: latest root scan head precedes finalized history"
        )));
    }
    if from > head.number {
        require_canonical_head(api, head.number, head.hash).await?;
        view.recheck(api.raw_provider()).await?;
        return Ok(false);
    }
    let start = ethereum_client::get_block(api.raw_provider(), from)
        .await
        .map_err(downcast_anyhow_to_ethereum_client)?;
    if start.header.number != from {
        return Err(ScanForEventsError::Other(anyhow::anyhow!(
            "HOLD: root scan start header height changed"
        )));
    }
    let events = api.fetch_merkle_roots_in_range(from, head.number).await?;
    let targets = root_inclusion_targets(&events, from, start.header.hash, head.number, head.hash)
        .map_err(ScanForEventsError::Other)?;
    let historical = targets
        .range(..=view.block_number())
        .map(|(&number, &hash)| (number, hash))
        .collect();
    view.verify_blocks(api.raw_provider(), &historical).await?;
    let head_number = head.number;
    let head_hash = head.hash;
    verify_hot_ancestry(api, view, head, &targets).await?;
    require_canonical_head(api, head_number, head_hash).await?;

    let queue = IMessageQueue::new(
        Address::from_slice(api.message_queue_address().as_bytes()),
        api.raw_provider(),
    );
    let witness_queue = IMessageQueue::new(
        Address::from_slice(api.message_queue_address().as_bytes()),
        api.raw_witness_provider()
            .map_err(ScanForEventsError::Other)?,
    );
    for observation in &events {
        let source = u32::try_from(observation.entry.block_number)
            .context("HOLD: source root height exceeds u32")
            .map_err(ScanForEventsError::Other)?;
        let stored_root = queue
            .getMerkleRoot(U256::from(source))
            .block(BlockId::hash_canonical(head_hash))
            .call()
            .await
            .map_err(ethereum_client::Error::ErrorDuringContractExecution)?
            .0;
        let witnessed_root = witness_queue
            .getMerkleRoot(U256::from(source))
            .block(BlockId::hash_canonical(head_hash))
            .call()
            .await
            .map_err(ethereum_client::Error::ErrorDuringContractExecution)?
            .0;
        if stored_root != witnessed_root {
            return Err(ScanForEventsError::Other(anyhow::anyhow!(
                "HOLD: Ethereum providers disagree on hot queue root"
            )));
        }
        validate_destination_root(
            observation,
            true, // Inclusion was authenticated against the pinned ancestry above.
            (stored_root != [0; 32]).then_some(stored_root),
        )
        .map_err(ScanForEventsError::Other)?;
        let matches = compare_source(&observation.entry).await?;
        metrics.merkle_roots_discovered_cnt.inc();
        if !matches {
            require_canonical_head(api, head_number, head_hash).await?;
            view.recheck(api.raw_provider()).await?;
            log::error!("Canonical queue root mismatch: {:?}", observation.entry);
            metrics.merkle_root_mismatch_cnt.inc();
            // A later unavailable root cannot revoke this authenticated mismatch.
            return Ok(true);
        }
    }
    require_canonical_head(api, head_number, head_hash).await?;
    view.recheck(api.raw_provider()).await?;
    Ok(false)
}

async fn verify_hot_ancestry(
    api: &EthApi,
    view: &FinalizedView,
    mut header: Header,
    targets: &BTreeMap<u64, TxHash>,
) -> Result<(), ScanForEventsError> {
    loop {
        if header.number < view.block_number()
            || header.inner.hash_slow() != header.hash
            || targets
                .get(&header.number)
                .is_some_and(|hash| *hash != header.hash)
        {
            return Err(ScanForEventsError::Other(anyhow::anyhow!(
                "HOLD: inconsistent hot root ancestry"
            )));
        }
        if header.number == view.block_number() {
            if header.hash != view.block_hash() {
                return Err(ScanForEventsError::Other(anyhow::anyhow!(
                    "HOLD: hot root ancestry conflicts with finalized history"
                )));
            }
            return Ok(());
        }
        let parent = api
            .raw_provider()
            .get_block_by_hash(header.parent_hash)
            .await
            .map_err(ethereum_client::Error::from)?
            .ok_or(ethereum_client::Error::ErrorFetchingBlock)?;
        if parent.header.number != header.number - 1 || parent.header.hash != header.parent_hash {
            return Err(ScanForEventsError::Other(anyhow::anyhow!(
                "HOLD: inconsistent hot root parent"
            )));
        }
        header = parent.header;
    }
}

async fn require_canonical_head(
    api: &EthApi,
    number: u64,
    hash: TxHash,
) -> Result<(), ScanForEventsError> {
    let current = ethereum_client::get_block(api.raw_provider(), number)
        .await
        .map_err(downcast_anyhow_to_ethereum_client)?;
    let witness = ethereum_client::get_block(
        api.raw_witness_provider()
            .map_err(ScanForEventsError::Other)?,
        number,
    )
    .await
    .map_err(downcast_anyhow_to_ethereum_client)?;
    if current.header.number != number
        || current.header.hash != hash
        || current.header.inner.hash_slow() != hash
        || witness.header.number != number
        || witness.header.hash != hash
        || witness.header.inner.hash_slow() != hash
        || witness.header.inner != current.header.inner
    {
        return Err(ScanForEventsError::Other(anyhow::anyhow!(
            "HOLD: root scan head was reorganized or differs between providers"
        )));
    }
    Ok(())
}

fn root_inclusion_targets(
    observations: &[MerkleRootObservation],
    from: u64,
    start_hash: TxHash,
    to: u64,
    head_hash: TxHash,
) -> anyhow::Result<BTreeMap<u64, TxHash>> {
    let mut targets = BTreeMap::from([(to, head_hash)]);
    for (number, hash) in std::iter::once((from, start_hash)).chain(
        observations
            .iter()
            .map(|observation| (observation.block_number, observation.block_hash)),
    ) {
        if let Some(old) = targets.insert(number, hash) {
            anyhow::ensure!(old == hash, "HOLD: conflicting root inclusion hashes");
        }
    }
    for observation in observations {
        anyhow::ensure!(
            !observation.removed && (from..=to).contains(&observation.block_number),
            "HOLD: removed or out-of-range root observation"
        );
    }
    Ok(targets)
}

fn validate_destination_root(
    observation: &MerkleRootObservation,
    canonical_inclusion: bool,
    stored_root: Option<[u8; 32]>,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        !observation.removed && canonical_inclusion,
        "HOLD: root event lacks authenticated canonical inclusion"
    );
    anyhow::ensure!(
        stored_root == Some(observation.entry.merkle_root.0),
        "HOLD: observed merkle root differs from pinned queue state"
    );
    Ok(())
}

fn source_height(
    event: &MerkleRootEntry,
    source_finalized: u32,
) -> Result<u32, ScanForEventsError> {
    let source =
        u32::try_from(event.block_number).context("HOLD: source root height exceeds u32")?;
    if source > source_finalized {
        return Err(
            anyhow::anyhow!("HOLD: root source block is not finalized on the source RPC").into(),
        );
    }
    Ok(source)
}

fn compare_source_root(
    event: &MerkleRootEntry,
    source_finalized: u32,
    source_root: anyhow::Result<(u32, H256)>,
) -> Result<bool, ScanForEventsError> {
    let expected_height = source_height(event, source_finalized)?;
    let (height, root) = source_root?;
    if height != expected_height {
        return Err(anyhow::anyhow!(
            "HOLD: historical source block height differs from requested height"
        )
        .into());
    }
    Ok(root == event.merkle_root)
}

fn downcast_anyhow_to_ethereum_client(err: anyhow::Error) -> ScanForEventsError {
    match err.downcast::<ethereum_client::Error>() {
        Ok(e) => ScanForEventsError::EthApi(e),
        Err(e) => ScanForEventsError::Other(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation() -> MerkleRootObservation {
        MerkleRootObservation {
            entry: MerkleRootEntry {
                block_number: 42,
                merkle_root: [7; 32].into(),
            },
            block_number: 10,
            block_hash: TxHash::repeat_byte(1),
            transaction_hash: TxHash::repeat_byte(2),
            transaction_index: 0,
            log_index: 0,
            removed: false,
        }
    }

    fn finish_scan(result: Result<bool, ScanForEventsError>) -> (State, Option<u64>, bool) {
        let mut state = State::ScanForEvents;
        let mut cursor = Some(10);
        let success = state.complete_scan(&mut cursor, 12, result).is_ok();
        (state, cursor, success)
    }

    #[test]
    fn exact_historical_match_advances_but_mismatch_challenges_without_advancing() {
        let event = observation().entry;
        let matched = compare_source_root(&event, 42, Ok((42, event.merkle_root)));
        assert_eq!(
            finish_scan(matched.map(|matches| !matches)),
            (State::ScanForEvents, Some(13), true)
        );
        let mismatch = compare_source_root(&event, 50, Ok((42, [8; 32].into())));
        assert_eq!(
            finish_scan(mismatch.map(|matches| !matches)),
            (State::ChallengeRoot { tx_hash: None }, Some(10), true)
        );
    }

    #[test]
    fn unavailable_historical_data_holds_cursor_without_challenging() {
        for message in [
            "Block #42 not present on RPC node",
            "State already discarded",
            "QueueMerkleRoot storage entry not found",
        ] {
            let result =
                compare_source_root(&observation().entry, 50, Err(anyhow::anyhow!(message)));
            assert_eq!(finish_scan(result), (State::ScanForEvents, Some(10), false));
        }
    }

    #[test]
    fn unfinalized_overflow_and_wrong_historical_height_hold_scan() {
        let mut event = observation().entry;
        for (block_number, finalized, historical_height) in [
            (42, 41, 42),
            (u64::from(u32::MAX) + 1, u32::MAX, 0),
            (42, 50, 43),
        ] {
            event.block_number = block_number;
            let result = compare_source_root(
                &event,
                finalized,
                Ok((historical_height, event.merkle_root)),
            );
            assert_eq!(finish_scan(result), (State::ScanForEvents, Some(10), false));
        }
    }

    #[test]
    fn removed_noncanonical_and_inconsistent_destination_logs_hold_scan() {
        let event = observation();
        assert!(validate_destination_root(&event, true, Some(event.entry.merkle_root.0)).is_ok());
        for (removed, included, stored_root) in [
            (true, true, Some(event.entry.merkle_root.0)),
            (false, false, Some(event.entry.merkle_root.0)),
            (false, true, Some([8; 32])),
            (false, true, None),
        ] {
            let result = validate_destination_root(
                &MerkleRootObservation { removed, ..event },
                included,
                stored_root,
            )
            .map(|()| true)
            .map_err(ScanForEventsError::Other);
            assert_eq!(finish_scan(result), (State::ScanForEvents, Some(10), false));
        }
    }

    #[test]
    fn inclusion_targets_reject_removed_out_of_range_and_conflicting_hashes() {
        let event = observation();
        let hash = event.block_hash;
        assert!(root_inclusion_targets(&[event], 10, hash, 12, hash).is_ok());
        for invalid in [
            MerkleRootObservation {
                removed: true,
                ..event
            },
            MerkleRootObservation {
                block_number: 13,
                ..event
            },
            MerkleRootObservation {
                block_number: 9,
                ..event
            },
            MerkleRootObservation {
                block_hash: TxHash::ZERO,
                ..event
            },
        ] {
            assert!(root_inclusion_targets(&[invalid], 10, hash, 12, hash).is_err());
        }
        assert!(root_inclusion_targets(&[], 10, hash, 10, TxHash::ZERO).is_err());
    }

    #[test]
    fn pending_and_uncertain_receipts_retain_original_hash_until_finalized() {
        let hash = TxHash::repeat_byte(3);
        let mut state = State::ChallengeRoot { tx_hash: None };
        state.record_submission(Ok(hash)).unwrap();
        for _ in 0..3 {
            assert!(!state.record_receipt(Ok(TxStatus::Pending)).unwrap());
            assert!(state
                .record_receipt(Err(ethereum_client::Error::ErrorFetchingBlock))
                .is_err());
            assert_eq!(
                state,
                State::ChallengeRoot {
                    tx_hash: Some(hash)
                }
            );
        }
        assert!(state.record_receipt(Ok(TxStatus::Finalized)).unwrap());
        assert_eq!(state, State::Exit);
    }

    #[test]
    fn transient_prebroadcast_failure_rescans_original_cursor_before_retry() {
        let mut cursor = Some(10);
        let mut state = State::ScanForEvents;
        state.complete_scan(&mut cursor, 12, Ok(true)).unwrap();
        let unavailable = ethereum_client::Error::ChallengeRootPreBroadcast(
            alloy::contract::Error::TransportError(
                alloy::transports::TransportErrorKind::http_error(503, String::new()),
            ),
        );
        assert!(state.record_submission(Err(unavailable)).is_err());
        assert_eq!(state, State::ScanForEvents);
        assert_eq!(cursor, Some(10));
        // A replacement branch with no mismatch must not receive a challenge.
        state.complete_scan(&mut cursor, 12, Ok(false)).unwrap();
        assert_eq!(state, State::ScanForEvents);
        assert_eq!(cursor, Some(13));
        // A mismatch still present after a fresh scan permits a new attempt.
        state.complete_scan(&mut cursor, 13, Ok(true)).unwrap();
        let hash = TxHash::repeat_byte(4);
        state.record_submission(Ok(hash)).unwrap();
        assert_eq!(
            state,
            State::ChallengeRoot {
                tx_hash: Some(hash)
            }
        );
    }

    #[test]
    fn ambiguous_send_and_permanent_prebroadcast_failures_require_operator() {
        let transport = || {
            alloy::contract::Error::TransportError(
                alloy::transports::TransportErrorKind::http_error(503, String::new()),
            )
        };
        let rejected =
            alloy::contract::Error::TransportError(alloy::transports::RpcError::ErrorResp(
                serde_json::from_str(r#"{"code":-32603,"message":"timeout"}"#).unwrap(),
            ));
        for error in [
            ethereum_client::Error::ErrorSendingTransaction(transport()),
            ethereum_client::Error::ChallengeRootPreBroadcast(rejected),
            ethereum_client::Error::ErrorFetchingBlock,
        ] {
            let mut state = State::ChallengeRoot { tx_hash: None };
            assert!(state.record_submission(Err(error)).is_err());
            assert_eq!(state, State::Exit);
        }
    }

    #[test]
    fn finalized_revert_requires_fresh_scan() {
        let mut state = State::ChallengeRoot {
            tx_hash: Some(TxHash::repeat_byte(3)),
        };
        assert!(!state.record_receipt(Ok(TxStatus::Failed)).unwrap());
        assert_eq!(state, State::ScanForEvents);
    }

    #[tokio::test]
    async fn loopback_missing_signer_holds_without_broadcast() {
        use actix_web::{web, App, HttpResponse, HttpServer};
        use serde_json::{json, Value};
        use std::{
            net::TcpListener,
            sync::{Arc, Mutex},
        };

        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = requests.clone();
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", socket.local_addr().unwrap());
        let server = HttpServer::new(move || {
            let requests = server_requests.clone();
            App::new().route(
                "/",
                web::post().to(move |request: web::Json<Value>| {
                    let requests = requests.clone();
                    async move {
                        let method = request["method"].as_str().unwrap();
                        requests.lock().unwrap().push(method.to_owned());
                        match method {
                            "eth_chainId" => HttpResponse::Ok().json(json!({
                                "jsonrpc": "2.0", "id": request["id"], "result": "0x88bb0"
                            })),
                            _ => HttpResponse::ServiceUnavailable().finish(),
                        }
                    }
                }),
            )
        })
        .workers(1)
        .listen(socket)
        .unwrap()
        .disable_signals()
        .run();
        let handle = server.handle();
        tokio::spawn(server);
        let queue = Address::repeat_byte(1);
        let api = EthApi::new(&endpoint, &format!("{queue:#x}"), None, None, None)
            .await
            .unwrap();
        let result = api.send_challenge_root().await;
        handle.stop(true).await;

        assert!(matches!(
            &result,
            Err(ethereum_client::Error::SignerNotConfigured)
        ));
        let mut state = State::ChallengeRoot { tx_hash: None };
        assert!(state.record_submission(result).is_err());
        assert_eq!(state, State::Exit);
        assert_eq!(*requests.lock().unwrap(), ["eth_chainId"]);
    }

    #[tokio::test]
    async fn hot_roots_are_revisited_and_mismatch_precedes_later_source_failure() {
        use actix_web::{web, App, HttpServer};
        use alloy::{rpc::types::Block, sol_types::SolEvent};
        use serde_json::{json, Value};
        use std::{
            net::TcpListener,
            sync::{
                atomic::{AtomicBool, AtomicUsize, Ordering},
                Arc,
            },
        };

        let mut chain = Vec::new();
        let mut parent = TxHash::ZERO;
        for number in 0..=4 {
            let mut block: Block = Block::default();
            block.header.inner.number = number;
            block.header.inner.parent_hash = parent;
            block.header.inner.timestamp = number * 12;
            if number == 0 {
                block.header.inner.extra_data = uuid::Uuid::new_v4().as_bytes().to_vec().into();
            }
            block.header.hash = block.header.inner.hash_slow();
            parent = block.header.hash;
            chain.push(block);
        }
        let mut fork = chain.clone();
        for number in 2..=4 {
            fork[number].header.inner.extra_data = vec![1].into();
            fork[number].header.inner.parent_hash = fork[number - 1].header.hash;
            fork[number].header.hash = fork[number].header.inner.hash_slow();
        }
        let chains = Arc::new([chain, fork]);
        let reorganized = Arc::new(AtomicBool::new(false));
        let witness_fault = Arc::new(AtomicUsize::new(0));
        let server_witness_fault = witness_fault.clone();
        let server_chains = chains.clone();
        let server_reorganized = reorganized.clone();
        let queue = Address::repeat_byte(1);
        let socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", socket.local_addr().unwrap());
        let witness_socket = TcpListener::bind("127.0.0.1:0").unwrap();
        let witness_endpoint = format!("http://{}/witness", witness_socket.local_addr().unwrap());
        let server = HttpServer::new(move || {
            let chains = server_chains.clone();
            let reorganized = server_reorganized.clone();
            let witness_fault = server_witness_fault.clone();
            App::new().route(
                "/{tail:.*}",
                web::post().to(move |http: actix_web::HttpRequest, request: web::Json<Value>| {
                    let chains = chains.clone();
                    let reorganized = reorganized.clone();
                    let witness_fault = witness_fault.clone();
                    let is_witness = http.path() == "/witness";
                    async move {
                        let is_fork = reorganized.load(Ordering::SeqCst);
                        let fault = if is_witness { witness_fault.load(Ordering::SeqCst) } else { 0 };
                        let chain = &chains[usize::from(is_fork)];
                        let params = &request["params"];
                        let root = |source| {
                            TxHash::repeat_byte(if is_fork && source == 42 { 8 } else { 7 })
                        };
                        let result = match request["method"].as_str().unwrap() {
                            "eth_chainId" => json!("0x7a69"),
                            "eth_getBlockByNumber" => {
                                let tag = params[0].as_str().unwrap();
                                let number = match tag {
                                    "finalized" => 1,
                                    "latest" => 4,
                                    _ => usize::from_str_radix(tag.trim_start_matches("0x"), 16).unwrap(),
                                };
                                if fault == 2 && number == 4 {
                                    json!(chains[0][number])
                                } else if witness_fault.load(Ordering::SeqCst) == 3 && number == 1 {
                                    let mut changed = chain[number].clone();
                                    changed.header.inner.extra_data = vec![9].into();
                                    changed.header.hash = changed.header.inner.hash_slow();
                                    json!(changed)
                                } else {
                                    json!(chain[number])
                                }
                            }
                            "eth_getBlockByHash" => chains.iter().flatten()
                                .find(|block| json!(block.header.hash) == params[0])
                                .map_or(Value::Null, |block| json!(block)),
                            "eth_getLogs" => {
                                assert_eq!(params[0]["toBlock"], "0x4");
                                let logs: Vec<_> = [(2, 42), (3, 43)].into_iter().map(|(number, source)| {
                                    let event = IMessageQueue::MerkleRoot {
                                        blockNumber: U256::from(source),
                                        merkleRoot: root(source),
                                    }.encode_log_data();
                                    json!({
                                        "address": queue, "topics": event.topics(), "data": event.data,
                                        "blockNumber": format!("0x{number:x}"),
                                        "blockHash": chain[number].header.hash,
                                        "transactionHash": TxHash::repeat_byte(source as u8),
                                        "transactionIndex": "0x0", "logIndex": "0x0", "removed": false,
                                    })
                                }).collect();
                                // Reconstructed from HTML / audited7fe1b032: primary omission.
                                if witness_fault.load(Ordering::SeqCst) == 4 && !is_witness {
                                    json!([])
                                } else {
                                    json!(logs)
                                }
                            }
                            "eth_call" => {
                                assert_eq!(params[1], json!({
                                    "blockHash": chain[4].header.hash, "requireCanonical": true,
                                }));
                                let input = params[0].get("input").or_else(|| params[0].get("data"))
                                    .unwrap().as_str().unwrap();
                                let source = u64::from_str_radix(&input[input.len() - 16..], 16).unwrap();
                                json!(if fault == 1 { TxHash::repeat_byte(9) } else { root(source) })
                            }
                            method => panic!("unexpected observer fixture RPC: {method}"),
                        };
                        web::Json(json!({"jsonrpc": "2.0", "id": request["id"], "result": result}))
                    }
                }),
            )
        })
        .workers(1)
        .listen(socket)
        .unwrap()
        .listen(witness_socket)
        .unwrap()
        .disable_signals()
        .run();
        let handle = server.handle();
        tokio::spawn(server);
        let beacon_chains = chains.clone();
        let beacon = ethereum_client::beacon_finality::test_fixtures::LocalBeaconFixture::new(
            31337,
            chains[0][0].header.hash,
            Arc::new(move || beacon_chains[0][1].header.clone()),
        )
        .await
        .unwrap();
        let api = EthApi::new(&endpoint, &format!("{queue:#x}"), None, None, None)
            .await
            .unwrap()
            .with_beacon_finality(beacon.config.clone(), &beacon.directory)
            .await
            .unwrap()
            .with_data_witness(&witness_endpoint, 31337, chains[0][0].header.hash)
            .await
            .unwrap();
        let mut view = api.verified_finalized_view().await.unwrap();
        let metrics = Metrics::new();
        let mut state = State::ScanForEvents;
        let mut cursor = Some(1);
        let matched = scan_roots(&api, &mut view, 1, &metrics, async |entry| {
            compare_source_root(entry, 50, Ok((entry.block_number as u32, [7; 32].into())))
        })
        .await;
        state
            .complete_scan(&mut cursor, view.block_number(), matched)
            .unwrap();
        assert_eq!((state, cursor), (State::ScanForEvents, Some(2)));

        // A primary-only hot fork or state answer must never authorize a challenge.
        reorganized.store(true, Ordering::SeqCst);
        for fault in [1, 2, 4] {
            witness_fault.store(fault, Ordering::SeqCst);
            state = State::ScanForEvents;
            let disagreement =
                scan_roots(&api, &mut view, cursor.unwrap(), &metrics, async |entry| {
                    compare_source_root(entry, 50, Ok((42, [7; 32].into())))
                })
                .await;
            assert!(state
                .complete_scan(&mut cursor, view.block_number(), disagreement)
                .is_err());
            assert_eq!((state, cursor), (State::ScanForEvents, Some(2)));
        }
        witness_fault.store(0, Ordering::SeqCst);

        // Recheck the Beacon-authenticated primary pin before challenge or cursor advance.
        for mismatch in [false, true] {
            state = State::ScanForEvents;
            let changed_pin = scan_roots(&api, &mut view, cursor.unwrap(), &metrics, async |_| {
                witness_fault.store(3, Ordering::SeqCst);
                Ok(!mismatch)
            })
            .await;
            assert!(state
                .complete_scan(&mut cursor, view.block_number(), changed_pin)
                .is_err());
            assert_eq!((state, cursor), (State::ScanForEvents, Some(2)));
            witness_fault.store(0, Ordering::SeqCst);
        }

        // Finality is still block 1. Revisit the replacement hot blocks, and do
        // not let unavailable state for the second root suppress the first.
        reorganized.store(true, Ordering::SeqCst);
        state = State::ScanForEvents;
        let mut checked = Vec::new();
        let mismatch = scan_roots(&api, &mut view, cursor.unwrap(), &metrics, async |entry| {
            checked.push(entry.block_number);
            let source = if entry.block_number == 43 {
                Err(anyhow::anyhow!("State already discarded"))
            } else {
                Ok((42, [7; 32].into()))
            };
            compare_source_root(entry, 50, source)
        })
        .await;
        state
            .complete_scan(&mut cursor, view.block_number(), mismatch)
            .unwrap();
        assert_eq!(checked, vec![42]);
        assert_eq!(
            (state, cursor),
            (State::ChallengeRoot { tx_hash: None }, Some(2))
        );

        // A reorg during the source read invalidates the pending mismatch.
        state = State::ScanForEvents;
        let orphaned = scan_roots(&api, &mut view, cursor.unwrap(), &metrics, async |entry| {
            reorganized.store(false, Ordering::SeqCst);
            compare_source_root(entry, 50, Ok((42, [7; 32].into())))
        })
        .await;
        assert!(state
            .complete_scan(&mut cursor, view.block_number(), orphaned)
            .is_err());
        assert_eq!((state, cursor), (State::ScanForEvents, Some(2)));

        state = State::ScanForEvents;
        let unavailable = scan_roots(&api, &mut view, cursor.unwrap(), &metrics, async |_| {
            Err(anyhow::anyhow!("State already discarded").into())
        })
        .await;
        assert!(state
            .complete_scan(&mut cursor, view.block_number(), unavailable)
            .is_err());
        assert_eq!((state, cursor), (State::ScanForEvents, Some(2)));

        // An explicit future start must not be moved backwards by a clean scan.
        state = State::ScanForEvents;
        cursor = Some(100);
        let future_start = scan_roots(&api, &mut view, 100, &metrics, async |_| {
            panic!("no roots before the configured start");
        })
        .await;
        state
            .complete_scan(&mut cursor, view.block_number(), future_start)
            .unwrap();
        assert_eq!((state, cursor), (State::ScanForEvents, Some(100)));
        handle.stop(true).await;
    }
}
