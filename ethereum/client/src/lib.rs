use alloy::{
    contract::Event,
    network::{Ethereum, EthereumWallet, TransactionBuilder},
    primitives::{Address, Bytes, B256, U256},
    providers::{
        fillers::{
            BlobGasFiller, ChainIdFiller, FillProvider, GasFiller, JoinFill, NonceFiller,
            SimpleNonceManager, WalletFiller,
        },
        Identity, PendingTransactionBuilder, Provider, ProviderBuilder, RootProvider,
    },
    pubsub::Subscription,
    rpc::types::{
        Block, BlockId, BlockNumberOrTag, Filter, Header, Log as RpcLog, TransactionReceipt,
    },
    signers::local::PrivateKeySigner,
    sol_types::SolEvent,
    transports::{ws::WsConnect, RpcError, TransportError, TransportErrorKind},
};
use anyhow::{bail, ensure, Context, Result as AnyResult};
use primitive_types::{H160, H256};
use reqwest::Url;
use std::{
    collections::{BTreeMap, HashMap},
    ops::Deref,
    str::FromStr,
    sync::{Arc, OnceLock, Weak},
    time::Duration,
};
use tokio::sync::{Mutex, OwnedMutexGuard};

pub use alloy::primitives::TxHash;
use serde::{Deserialize, Serialize};
use serde_json::Value;
mod finality_archive;
use tokio::time::{timeout, Instant};

const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn connect_ws(ws: WsConnect) -> Result<alloy::rpc::client::RpcClient, TransportError> {
    timeout(
        RPC_REQUEST_TIMEOUT,
        alloy::rpc::client::RpcClient::builder()
            .layer(tower::util::MapFutureLayer::new(
                |future| -> alloy::transports::TransportFut<'static> { Box::pin(future) },
            ))
            .layer(tower::util::MapErrLayer::new(
                |error: tower::BoxError| match error.downcast::<TransportError>() {
                    Ok(error) => *error,
                    Err(error) => RpcError::Transport(TransportErrorKind::Custom(error)),
                },
            ))
            .layer(tower::timeout::TimeoutLayer::new(RPC_REQUEST_TIMEOUT))
            .ws(ws),
    )
    .await
    .map_err(|_| TransportErrorKind::custom_str("Ethereum WebSocket connection timed out"))?
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransactionIdentity {
    pub hash: B256,
    pub from: Address,
    pub nonce: u64,
    pub to: Option<Address>,
    pub block_number: Option<u64>,
}

fn quantity(value: &Value, field: &str) -> AnyResult<u64> {
    let number = value
        .as_str()
        .and_then(|number| number.strip_prefix("0x"))
        .with_context(|| format!("transaction {field} is not a hex quantity"))?;
    ensure!(
        !number.is_empty() && (number == "0" || !number.starts_with('0')),
        "transaction {field} is not a canonical hex quantity"
    );
    u64::from_str_radix(number, 16).with_context(|| format!("invalid transaction {field}"))
}

fn decode_transaction_identity(
    value: &Value,
    expected_hash: B256,
) -> AnyResult<TransactionIdentity> {
    let fields = value
        .as_object()
        .context("transaction identity is not an object")?;
    let field = |name| -> AnyResult<&str> {
        fields
            .get(name)
            .and_then(Value::as_str)
            .with_context(|| format!("transaction identity {name} is missing or invalid"))
    };
    let hash = field("hash")?.parse()?;
    let from = field("from")?.parse()?;
    let nonce = quantity(
        fields
            .get("nonce")
            .context("transaction nonce is missing")?,
        "nonce",
    )?;
    let to = match fields
        .get("to")
        .context("transaction identity to is missing")?
    {
        Value::Null => None,
        Value::String(to) => Some(to.parse().context("invalid transaction destination")?),
        _ => bail!("invalid transaction destination"),
    };
    let block_number = match fields.get("blockNumber") {
        None | Some(Value::Null) => None,
        Some(value) => Some(quantity(value, "blockNumber")?),
    };
    ensure!(
        hash == expected_hash,
        "RPC returned a different transaction hash"
    );
    Ok(TransactionIdentity {
        hash,
        from,
        nonce,
        to,
        block_number,
    })
}

/// Identity-only lookup: full Alloy RPC transaction decoding rejects sparse Hoodi responses.
pub async fn transaction_identity(
    provider: &impl Provider,
    hash: B256,
) -> AnyResult<Option<TransactionIdentity>> {
    let value: Option<Value> = provider
        .raw_request("eth_getTransactionByHash".into(), (hash,))
        .await?;
    value
        .as_ref()
        .map(|value| decode_transaction_identity(value, hash))
        .transpose()
}

pub mod abi;
use abi::{
    BridgingPayment, IERC20Manager, IMessageQueue,
    IMessageQueue::{IMessageQueueInstance, MerkleRoot, VaraMessage},
};

pub mod error;
pub use error::Error;

#[derive(Debug)]
pub struct ContentMessageSubmissionError {
    pub error: Error,
    /// Ethereum account nonce selected for this submission, when allocation succeeded.
    pub nonce: Option<u64>,
}

/// Holds exclusive ownership of Ethereum account submissions across retries.
pub struct SubmissionGuard {
    _guard: OwnedMutexGuard<()>,
    sender: Address,
    lock: Arc<Mutex<()>>,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PreparedContentMessage {
    pub chain_id: u64,
    pub contract: Address,
    pub sender: Address,
    pub nonce: u64,
    pub raw_transaction: Vec<u8>,
    pub hash: TxHash,
}

pub fn prepared_content_message_identity(
    prepared: &PreparedContentMessage,
) -> AnyResult<TransactionIdentity> {
    use alloy::{
        consensus::{transaction::SignerRecoverable, Transaction, TxEnvelope},
        eips::Decodable2718,
    };
    ensure!(
        alloy::primitives::keccak256(&prepared.raw_transaction) == prepared.hash,
        "prepared transaction hash changed"
    );
    let mut raw = prepared.raw_transaction.as_slice();
    let transaction =
        TxEnvelope::decode_2718(&mut raw).context("invalid prepared signed transaction")?;
    ensure!(
        raw.is_empty()
            && transaction.nonce() == prepared.nonce
            && transaction.to() == Some(prepared.contract)
            && transaction.chain_id() == Some(prepared.chain_id),
        "prepared transaction nonce, chain or contract changed"
    );
    let from = transaction
        .recover_signer()
        .context("invalid prepared transaction signature")?;
    ensure!(
        from == prepared.sender,
        "prepared transaction signer changed"
    );
    Ok(TransactionIdentity {
        hash: prepared.hash,
        from,
        nonce: prepared.nonce,
        to: Some(prepared.contract),
        block_number: None,
    })
}

// Every EthApi for the same signer and chain coordinates nonce allocation, including
// independently constructed instances in this process. Weak entries avoid
// retaining one lock per signer forever.
type SubmissionAccount = (u64, Address);
type SubmissionLocks = HashMap<SubmissionAccount, Weak<Mutex<()>>>;

static SUBMISSION_LOCKS: OnceLock<std::sync::Mutex<SubmissionLocks>> = OnceLock::new();

fn submission_lock(chain_id: u64, public_key: Address) -> Arc<Mutex<()>> {
    let registry = SUBMISSION_LOCKS.get_or_init(Default::default);
    let mut locks = registry
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);

    locks.retain(|_, lock| lock.strong_count() != 0);
    let key = (chain_id, public_key);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }

    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

// 2 Gwei
const MAX_FEE_PER_GAS: u128 = 2_000_000_000;
// 0.5 Gwei
const MAX_PRIORITY_FEE_PER_GAS: u128 = 500_000_000;

type ProviderFillers = JoinFill<
    JoinFill<
        JoinFill<JoinFill<Identity, GasFiller>, BlobGasFiller>,
        NonceFiller<SimpleNonceManager>,
    >,
    ChainIdFiller,
>;

type ProviderType =
    FillProvider<JoinFill<ProviderFillers, WalletFiller<EthereumWallet>>, RootProvider<Ethereum>>;

#[derive(Clone)]
pub struct Contracts {
    provider: ProviderType,
    message_queue_instance: IMessageQueueInstance<ProviderType, Ethereum>,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
}

#[derive(Debug, Clone)]
pub struct MerkleRootEntry {
    pub block_number: u64,
    pub merkle_root: H256,
}

#[derive(Debug, Clone)]
pub struct DepositEventEntry {
    pub from: H160,
    pub to: H256,
    pub token: H160,
    pub amount: primitive_types::U256,

    pub tx_hash: TxHash,
}

#[derive(Debug)]
pub enum TxStatus {
    Finalized,
    Pending,
    Failed,
}

#[derive(Debug)]
pub struct FinalizedTransactionReceipt {
    pub receipt: TransactionReceipt,
    pub included_block_number: u64,
    pub included_block_hash: B256,
    pub finalized_block_number: u64,
    pub finalized_block_hash: B256,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FinalizedSubmissionSnapshot {
    pub block_number: u64,
    pub block_hash: B256,
    pub pinned_nonce_consumed: bool,
    pub message_processed: bool,
}

#[derive(Clone)]
pub struct PollingEthApi {
    provider: RootProvider,
    url: Url,
    ws_max_retry: Option<u32>,
    ws_retry_interval: Option<Duration>,
}

impl PollingEthApi {
    pub async fn new(url: &str) -> AnyResult<Self> {
        Self::new_with_retries(url, None, None).await
    }

    pub async fn new_with_retries(
        url: &str,
        ws_max_retry: Option<u32>,
        ws_retry_interval: Option<Duration>,
    ) -> AnyResult<Self> {
        let url = Url::parse(url).context("Provided url is not valid")?;
        let provider = match url.scheme() {
            "http" | "https" => RootProvider::builder().connect_http(url.clone()),
            "ws" | "wss" => {
                let mut ws = WsConnect::new(url.clone());
                if let Some(ws_max_retry) = ws_max_retry {
                    ws = ws.with_max_retries(ws_max_retry);
                }
                if let Some(ws_retry_interval) = ws_retry_interval {
                    ws = ws.with_retry_interval(ws_retry_interval);
                }
                RootProvider::builder().connect_client(connect_ws(ws).await?)
            }
            scheme => anyhow::bail!("Unsupported Ethereum RPC URL scheme: {scheme}"),
        };

        Ok(Self {
            provider,
            url,
            ws_max_retry,
            ws_retry_interval,
        })
    }

    pub async fn reconnect(&self) -> AnyResult<Self> {
        Self::new_with_retries(self.url.as_str(), self.ws_max_retry, self.ws_retry_interval).await
    }
    pub async fn verified_finalized_view(&self) -> Result<FinalizedView, Error> {
        verified_finalized_view(&self.provider).await
    }

    pub async fn enable_finality_archive(
        &self,
        directory: &std::path::Path,
        expected_genesis: B256,
    ) -> AnyResult<()> {
        enable_finality_archive(&self.provider, directory, expected_genesis).await
    }

    pub async fn is_finalized_block(&self, number: u64, hash: H256) -> AnyResult<bool> {
        Ok(
            finalized_ancestor(&self.provider, number, B256::from(hash.0))
                .await?
                .is_some(),
        )
    }

    pub async fn chain_id(&self) -> AnyResult<u64> {
        Ok(self.provider.get_chain_id().await?)
    }

    pub async fn finalized_block(&self) -> AnyResult<Block> {
        self::finalized_block(&self.provider).await
    }

    /// Returns a receipt only after its included block is canonical and finalized.
    pub async fn get_finalized_receipt(
        &self,
        tx_hash: TxHash,
    ) -> AnyResult<Option<FinalizedTransactionReceipt>> {
        Ok(finalized_transaction_receipt(&self.provider, tx_hash).await?)
    }
    pub async fn get_transaction_block_number(&self, hash: TxHash) -> AnyResult<Option<u64>> {
        transaction_identity(&self.provider, hash)
            .await?
            .map(|tx| tx.block_number.context("Block number is None"))
            .transpose()
    }

    pub async fn safe_block(&self) -> AnyResult<Block> {
        self::safe_block(&self.provider).await
    }

    pub async fn get_block(&self, block: u64) -> AnyResult<Block> {
        self::get_block(&self.provider, block).await
    }

    pub async fn fetch_fee_paid_events_txs(
        &self,
        contract_address: H160,
        block: u64,
    ) -> AnyResult<Vec<TxHash>> {
        let filter = Filter::new()
            .address(Address::from_slice(contract_address.as_bytes()))
            .event_signature(BridgingPayment::FeePaid::SIGNATURE_HASH)
            .from_block(block)
            .to_block(block);

        let event: Event<_, BridgingPayment::FeePaid, Ethereum> =
            Event::new(self.provider.clone(), filter);

        let logs = event.query().await.context("Failed to query event")?;

        logs.into_iter()
            .map(|(_, log)| log.transaction_hash.context("Failed to fetch transaction"))
            .collect()
    }

    pub async fn fetch_fee_paid_events_txs_at(
        &self,
        contract_address: H160,
        block: u64,
        hash: H256,
    ) -> AnyResult<Vec<TxHash>> {
        let filter = Filter::new()
            .address(Address::from_slice(contract_address.as_bytes()))
            .event_signature(BridgingPayment::FeePaid::SIGNATURE_HASH)
            .at_block_hash(B256::from(hash.0));
        let event: Event<_, BridgingPayment::FeePaid, Ethereum> =
            Event::new(self.provider.clone(), filter);
        event
            .query()
            .await?
            .into_iter()
            .map(|(_, log)| {
                ensure!(
                    log.block_number == Some(block)
                        && log.block_hash == Some(B256::from(hash.0))
                        && !log.removed,
                    "fee discovery returned a removed or differently included log; HOLD"
                );
                log.transaction_hash
                    .context("fee log has no original transaction hash")
            })
            .collect()
    }

    pub async fn fetch_deposit_events_at(
        &self,
        contract_address: H160,
        block: u64,
        hash: H256,
    ) -> AnyResult<Vec<DepositEventEntry>> {
        let filter = Filter::new()
            .address(Address::from_slice(contract_address.as_bytes()))
            .event_signature(IERC20Manager::BridgingRequested::SIGNATURE_HASH)
            .at_block_hash(B256::from(hash.0));
        let event: Event<_, IERC20Manager::BridgingRequested, Ethereum> =
            Event::new(self.provider.clone(), filter);
        event
            .query()
            .await?
            .into_iter()
            .map(|(deposit, log)| {
                ensure!(
                    log.block_number == Some(block)
                        && log.block_hash == Some(B256::from(hash.0))
                        && !log.removed,
                    "deposit discovery returned a removed or differently included log; HOLD"
                );
                Ok(DepositEventEntry {
                    from: H160(deposit.from.0 .0),
                    to: H256(deposit.to.0),
                    token: H160(deposit.token.0 .0),
                    amount: primitive_types::U256::from_little_endian(
                        &deposit.amount.to_le_bytes_vec(),
                    ),
                    tx_hash: log
                        .transaction_hash
                        .context("deposit log has no original transaction hash")?,
                })
            })
            .collect()
    }

    pub async fn fetch_deposit_events(
        &self,
        contract_address: H160,
        block: u64,
    ) -> AnyResult<Vec<DepositEventEntry>> {
        let filter = Filter::new()
            .address(Address::from_slice(contract_address.as_bytes()))
            .event_signature(IERC20Manager::BridgingRequested::SIGNATURE_HASH)
            .from_block(block)
            .to_block(block);

        let event: Event<_, IERC20Manager::BridgingRequested, Ethereum> =
            Event::new(self.provider.clone(), filter);

        let logs = event.query().await.context("Failed to query event")?;

        logs.into_iter()
            .map(
                |(
                    IERC20Manager::BridgingRequested {
                        from,
                        to,
                        token,
                        amount,
                    },
                    log,
                )| {
                    let tx_hash = log
                        .transaction_hash
                        .context("Failed to fetch transaction")?;

                    Ok(DepositEventEntry {
                        from: H160(*from.0),
                        to: H256(to.0),
                        token: H160(*token.0),
                        amount: primitive_types::U256::from_little_endian(
                            &amount.to_le_bytes_vec(),
                        ),
                        tx_hash,
                    })
                },
            )
            .collect()
    }
}

impl Deref for PollingEthApi {
    type Target = RootProvider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
}

pub async fn finalized_block(provider: impl Provider) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Finalized)
        .await?
        .context("Finalized block is None")
}

pub async fn latest_block(provider: impl Provider) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .context("Latest block is None")
}

pub async fn safe_block(provider: impl Provider) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Safe)
        .await?
        .context("Safe block is None")
}

pub async fn get_block(provider: impl Provider, block: u64) -> AnyResult<Block> {
    provider
        .get_block_by_number(BlockNumberOrTag::Number(block))
        .await?
        .context("Block is None")
}

// Each hash is authenticated by canonical RLP headers linked to a freshly pinned
// finalized head. Numbered RPC reads are hints, never finality authority.
struct VerifiedAncestry {
    head: (u64, B256),
    lowest: (u64, B256),
    known: BTreeMap<u64, B256>,
    required_heads: BTreeMap<u64, B256>,
    // Interrupted interior descents must survive the bounded recent window.
    frontiers: BTreeMap<u64, (u64, B256)>,
}

static FINALIZED_ANCESTRY: OnceLock<std::sync::Mutex<HashMap<B256, VerifiedAncestry>>> =
    OnceLock::new();
static FINALITY_ARCHIVES: OnceLock<
    std::sync::Mutex<HashMap<B256, Arc<finality_archive::HeaderArchive>>>,
> = OnceLock::new();
static FINALITY_LOCKS: OnceLock<std::sync::Mutex<HashMap<B256, Weak<Mutex<()>>>>> = OnceLock::new();
const MAX_CHAINS: usize = 8;
const MAX_KNOWN_ANCESTORS: usize = 32_768;
const MAX_PROOF_DURATION: Duration = Duration::from_secs(300);

impl VerifiedAncestry {
    fn remember(&mut self, proof: impl IntoIterator<Item = (u64, B256)>) {
        self.known.extend(proof);
        while self.known.len() > MAX_KNOWN_ANCESTORS {
            self.known.pop_first();
        }
    }
}

fn archive_for(genesis: B256) -> Option<Arc<finality_archive::HeaderArchive>> {
    FINALITY_ARCHIVES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&genesis)
        .cloned()
}

fn finality_lock(genesis: B256) -> Arc<Mutex<()>> {
    let mut locks = FINALITY_LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    locks.retain(|_, lock| lock.strong_count() != 0);
    if let Some(lock) = locks.get(&genesis).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(genesis, Arc::downgrade(&lock));
    lock
}

fn register_archive(directory: &std::path::Path, genesis: B256) -> AnyResult<()> {
    let mut archives = FINALITY_ARCHIVES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(existing) = archives.get(&genesis) {
        ensure!(
            existing.directory() == directory,
            "finality archive directory changed within one owner"
        );
    } else {
        archives.insert(
            genesis,
            Arc::new(finality_archive::HeaderArchive::open(directory)?),
        );
    }
    Ok(())
}

async fn enable_finality_archive(
    provider: &impl Provider,
    directory: &std::path::Path,
    expected_genesis: B256,
) -> AnyResult<()> {
    let genesis = provider
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await?
        .context("missing archive chain genesis")?;
    ensure!(
        valid_header(&genesis, 0, expected_genesis),
        "finality archive network/genesis mismatch"
    );
    register_archive(directory, expected_genesis)
}

fn valid_header(block: &Block, number: u64, hash: B256) -> bool {
    block.header.number == number
        && block.header.hash == hash
        && block.header.inner.hash_slow() == hash
}

async fn finality_pin(provider: &impl Provider) -> Result<(Header, B256), Error> {
    let head = timeout(
        Duration::from_secs(10),
        provider.get_block_by_number(BlockNumberOrTag::Finalized),
    )
    .await
    .map_err(|_| Error::FinalizedAncestryPending)??
    .ok_or(Error::FinalizedAncestryPending)?;
    let genesis = timeout(
        Duration::from_secs(10),
        provider.get_block_by_number(BlockNumberOrTag::Number(0)),
    )
    .await
    .map_err(|_| Error::FinalizedAncestryPending)??
    .ok_or(Error::FinalizedAncestryPending)?;
    if !valid_header(&head, head.header.number, head.header.hash)
        || !valid_header(&genesis, 0, genesis.header.hash)
        || (head.header.number == 0 && head.header.hash != genesis.header.hash)
    {
        return Err(Error::FinalizedAncestryPending);
    }
    Ok((head.header, genesis.header.hash))
}

async fn walk_ancestors(
    provider: &impl Provider,
    start: (u64, B256),
    target: u64,
    started: Instant,
    archive: Option<&finality_archive::HeaderArchive>,
    targets: Option<&BTreeMap<u64, B256>>,
) -> Result<(u64, B256, Vec<(u64, B256)>), Error> {
    use std::collections::VecDeque;
    let (mut number, mut hash) = start;
    let mut verified = VecDeque::new();
    let mut requests = tokio::task::JoinSet::new();
    let mut ready = BTreeMap::new();
    let mut next_request = number;
    let mut local = VecDeque::new();
    let mut chunk = Vec::with_capacity(512);
    while number > target && started.elapsed() < MAX_PROOF_DURATION {
        if local.is_empty() {
            if let Some(archive) = archive {
                let stored = archive
                    .read(number, hash)
                    .map_err(|_| Error::FinalizedAncestryPending)?
                    .unwrap_or_default();
                if !stored.is_empty() {
                    // A local chunk can jump past every speculative numbered
                    // hint. Cancel them before opening another bounded window.
                    requests.shutdown().await;
                    ready.clear();
                    next_request = number;
                    if !chunk.is_empty() {
                        archive
                            .save(&chunk)
                            .map_err(|_| Error::FinalizedAncestryPending)?;
                        chunk.clear();
                    }
                }
                local = stored.into();
            }
        }
        let header = if let Some(header) = local.pop_front() {
            if header.number != number || header.hash_slow() != hash {
                return Err(Error::FinalizedAncestryPending);
            }
            header
        } else {
            // No more than 32 in-flight or completed numbered hints.
            while next_request > target && number.saturating_sub(next_request) < 32 {
                let requested = next_request;
                let provider = provider.root().clone();
                requests.spawn(async move {
                    (
                        requested,
                        timeout(
                            Duration::from_secs(10),
                            provider.get_block_by_number(BlockNumberOrTag::Number(requested)),
                        )
                        .await,
                    )
                });
                next_request -= 1;
            }
            let observed = loop {
                if let Some(observed) = ready.remove(&number) {
                    break observed;
                }
                match requests.join_next().await {
                    Some(Ok((height, observed))) if height == number => break observed,
                    Some(Ok((height, observed))) => {
                        ready.insert(height, observed);
                    }
                    _ => break Ok(Ok(None)),
                }
            };
            let block = match observed {
                Ok(Ok(Some(block))) if valid_header(&block, number, hash) => Some(block),
                Ok(Ok(_)) => timeout(Duration::from_secs(10), provider.get_block_by_hash(hash))
                    .await
                    .ok()
                    .and_then(Result::ok)
                    .flatten()
                    .filter(|block| valid_header(block, number, hash)),
                _ => None,
            };
            let Some(block) = block else { break };
            // Persist chunks as work is authenticated, including unfinished gaps.
            chunk.push(block.header.inner.clone());
            if chunk.len() == 512 {
                if let Some(archive) = archive {
                    archive
                        .save(&chunk)
                        .map_err(|_| Error::FinalizedAncestryPending)?;
                }
                chunk.clear();
            }
            block.header.inner
        };
        if let Some(targets) = targets {
            if let Some(expected) = targets.get(&number) {
                if *expected != hash {
                    return Err(Error::FinalizedAncestryConflict);
                }
            }
        }
        number -= 1;
        hash = header.parent_hash;
        if let Some(targets) = targets {
            if let Some(expected) = targets.get(&number) {
                if *expected != hash {
                    return Err(Error::FinalizedAncestryConflict);
                }
            }
        }
        verified.push_back((number, hash));
        if verified.len() > MAX_KNOWN_ANCESTORS {
            verified.pop_front();
        }
        if local.is_empty() && next_request > number {
            next_request = number;
        }
        // Discard obsolete hints if local material jumped over their range.
        ready.retain(|height, _| *height <= number);
    }
    if let Some(archive) = archive {
        archive
            .save(&chunk)
            .map_err(|_| Error::FinalizedAncestryPending)?;
    }
    Ok((number, hash, verified.into()))
}

/// Immutable, continuity-checked state pin. State-only completion legitimately
/// permits another transaction to process a message; it never attributes that
/// success to an unproved original receipt.
#[derive(Clone, Debug)]
pub struct FinalizedView {
    header: Header,
    genesis: B256,
    blocks: BTreeMap<u64, B256>,
}

impl FinalizedView {
    pub fn block_number(&self) -> u64 {
        self.header.number
    }
    pub fn block_hash(&self) -> B256 {
        self.header.hash
    }
    pub fn genesis_hash(&self) -> B256 {
        self.genesis
    }
    pub fn block_id(&self) -> BlockId {
        BlockId::hash_canonical(self.header.hash)
    }
    pub fn contains(&self, number: u64, hash: B256) -> bool {
        self.blocks.get(&number) == Some(&hash)
    }

    /// Prove the complete distinct historical target set in one descending pass.
    /// Sparse targets survive the recent-header bound; archive chunks bound RAM.
    pub async fn verify_blocks(
        &mut self,
        provider: &impl Provider,
        targets: &BTreeMap<u64, B256>,
    ) -> Result<(), Error> {
        let _verification = finality_lock(self.genesis).lock_owned().await;
        let Some((&oldest, _)) = targets.first_key_value() else {
            return Ok(());
        };
        if targets
            .last_key_value()
            .is_some_and(|(&number, _)| number > self.block_number())
        {
            return Err(Error::FinalizedAncestryPending);
        }
        let archive = archive_for(self.genesis);
        if let Some(expected) = targets.get(&self.block_number()) {
            if *expected != self.block_hash() {
                return Err(Error::FinalizedAncestryConflict);
            }
        }
        let (number, hash, proof) = walk_ancestors(
            provider,
            (self.block_number(), self.block_hash()),
            oldest,
            Instant::now(),
            archive.as_deref(),
            Some(targets),
        )
        .await?;
        if number != oldest {
            return Err(Error::FinalizedAncestryPending);
        }
        if targets.get(&number) != Some(&hash) {
            return Err(Error::FinalizedAncestryConflict);
        }
        let state = FINALIZED_ANCESTRY.get_or_init(Default::default);
        let mut state = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cache = state
            .get_mut(&self.genesis)
            .filter(|cache| {
                cache.head == (self.block_number(), self.block_hash())
                    && cache.required_heads.is_empty()
            })
            .ok_or(Error::FinalizedAncestryPending)?;
        cache.remember(proof);
        cache.lowest = (number, hash);
        self.blocks
            .extend(targets.iter().map(|(&number, &hash)| (number, hash)));
        Ok(())
    }
}

async fn verified_finalized_view(provider: &impl Provider) -> Result<FinalizedView, Error> {
    let (header, genesis) = finality_pin(provider).await?;
    let head = (header.number, header.hash);
    match finalized_ancestor_at(provider, head.0, head.1, &header, genesis).await? {
        Some(_) => Ok(FinalizedView {
            header,
            genesis,
            blocks: BTreeMap::from([head]),
        }),
        None => Err(Error::FinalizedAncestryConflict),
    }
}

async fn finalized_ancestor(
    provider: &impl Provider,
    number: u64,
    hash: B256,
) -> Result<Option<(u64, B256)>, Error> {
    let (header, genesis) = finality_pin(provider).await?;
    finalized_ancestor_at(provider, number, hash, &header, genesis).await
}

async fn finalized_ancestor_at(
    provider: &impl Provider,
    number: u64,
    hash: B256,
    header: &Header,
    genesis: B256,
) -> Result<Option<(u64, B256)>, Error> {
    let _verification = finality_lock(genesis).lock_owned().await;
    let started = Instant::now();
    let head = (header.number, header.hash);
    if number > head.0 {
        return Err(Error::FinalizedAncestryPending);
    }
    let archive = archive_for(genesis);
    let durable_required = if let Some(archive) = archive.as_ref() {
        archive
            .witness(genesis, &header.inner)
            .map_err(|_| Error::FinalizedAncestryPending)?
    } else {
        Vec::new()
    };
    let state = FINALIZED_ANCESTRY.get_or_init(Default::default);
    let previous = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&genesis)
        .map(|cached| (cached.head, cached.required_heads.len()));
    if let Some((old_head, required_count)) = previous {
        if head != old_head && required_count >= MAX_KNOWN_ANCESTORS {
            return Err(Error::FinalizedAncestryPending);
        }
        if head.0 < old_head.0 {
            return Err(Error::FinalizedAncestryPending);
        }
        if head != old_head {
            let (height, ancestor, extension) = walk_ancestors(
                provider,
                head,
                old_head.0,
                started,
                archive.as_deref(),
                None,
            )
            .await?;
            let mut cache = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let cache = cache
                .get_mut(&genesis)
                .filter(|cache| cache.head == old_head)
                .ok_or(Error::FinalizedAncestryPending)?;
            if height == old_head.0 && ancestor != old_head.1 {
                return Ok(None);
            }
            if height > old_head.0 {
                if cache.required_heads.len() >= MAX_KNOWN_ANCESTORS {
                    return Err(Error::FinalizedAncestryPending);
                }
                cache.head = head;
                cache.lowest = (height, ancestor);
                cache.required_heads.insert(old_head.0, old_head.1);
                cache.known.insert(head.0, head.1);
                cache.remember(extension);
                return Err(Error::FinalizedAncestryPending);
            }
            cache.head = head;
            cache.remember(std::iter::once(head).chain(extension));
        }
    } else {
        let mut cache = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !cache.contains_key(&genesis) {
            if cache.len() >= MAX_CHAINS {
                let evicted = cache
                    .iter()
                    .find_map(|(&chain, ancestry)| {
                        (ancestry.required_heads.is_empty() && ancestry.frontiers.is_empty())
                            .then_some(chain)
                    })
                    .ok_or(Error::FinalizedAncestryPending)?;
                cache.remove(&evicted);
            }
            cache.insert(
                genesis,
                VerifiedAncestry {
                    head,
                    lowest: head,
                    known: BTreeMap::from([head]),
                    required_heads: Default::default(),
                    frontiers: Default::default(),
                },
            );
        }
    }
    let pending = {
        let cache = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cache = cache
            .get(&genesis)
            .filter(|cache| cache.head == head)
            .ok_or(Error::FinalizedAncestryPending)?;
        cache
            .required_heads
            .first_key_value()
            .map(|(&height, &hash)| (cache.lowest, (height, hash)))
    };
    if let Some((start, required)) = pending {
        let required_heads = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&genesis)
            .ok_or(Error::FinalizedAncestryPending)?
            .required_heads
            .clone();
        let (height, ancestor, links) = walk_ancestors(
            provider,
            start,
            required.0,
            started,
            archive.as_deref(),
            Some(&required_heads),
        )
        .await?;
        let mut cache = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cache = cache
            .get_mut(&genesis)
            .filter(|cache| cache.head == head)
            .ok_or(Error::FinalizedAncestryPending)?;
        cache.lowest = (height, ancestor);
        cache.remember(links);
        if height != required.0 {
            return Err(Error::FinalizedAncestryPending);
        }
        if ancestor != required.1 {
            return Ok(None);
        }
        cache.required_heads.clear();
    }
    if let Some(&(oldest, _)) = durable_required.iter().min_by_key(|tip| tip.0) {
        let targets: BTreeMap<_, _> = durable_required.into_iter().collect();
        let (height, ancestor, links) = walk_ancestors(
            provider,
            head,
            oldest,
            started,
            archive.as_deref(),
            Some(&targets),
        )
        .await?;
        if height != oldest {
            let mut cache = state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let cache = cache
                .get_mut(&genesis)
                .filter(|cache| cache.head == head)
                .ok_or(Error::FinalizedAncestryPending)?;
            cache.required_heads.extend(targets);
            cache.lowest = (height, ancestor);
            cache.remember(links);
            return Err(Error::FinalizedAncestryPending);
        }
        if targets.get(&height) != Some(&ancestor) {
            return Ok(None);
        }
        let mut cache = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cache = cache
            .get_mut(&genesis)
            .filter(|cache| cache.head == head && cache.required_heads.is_empty())
            .ok_or(Error::FinalizedAncestryPending)?;
        cache.remember(links);
    }
    let start = {
        let cache = state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let cache = cache
            .get(&genesis)
            .filter(|cache| cache.head == head && cache.required_heads.is_empty())
            .ok_or(Error::FinalizedAncestryPending)?;
        cache.frontiers.get(&number).copied().unwrap_or_else(|| {
            if number <= cache.lowest.0 {
                cache.lowest
            } else {
                cache
                    .known
                    .range(number..)
                    .next()
                    .map(|(&height, &ancestor)| (height, ancestor))
                    .unwrap_or(head)
            }
        })
    };
    let (height, ancestor, proof) =
        walk_ancestors(provider, start, number, started, archive.as_deref(), None).await?;
    let mut cache = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let cache = cache
        .get_mut(&genesis)
        .filter(|cache| cache.head == head && cache.required_heads.is_empty())
        .ok_or(Error::FinalizedAncestryPending)?;
    if height < cache.lowest.0 {
        cache.lowest = (height, ancestor);
    }
    cache.remember(proof);
    if height != number {
        if !cache.frontiers.contains_key(&number) && cache.frontiers.len() >= MAX_KNOWN_ANCESTORS {
            return Err(Error::FinalizedAncestryPending);
        }
        cache.frontiers.insert(number, (height, ancestor));
        return Err(Error::FinalizedAncestryPending);
    }
    cache.frontiers.remove(&number);
    if ancestor != hash {
        return Ok(None);
    }
    if let Some(archive) = archive {
        archive
            .complete(genesis, &header.inner)
            .map_err(|_| Error::FinalizedAncestryPending)?;
    }
    Ok(Some(head))
}
async fn finalized_transaction_receipt(
    provider: impl Provider,
    tx_hash: TxHash,
) -> Result<Option<FinalizedTransactionReceipt>, Error> {
    let Some(receipt) = provider.get_transaction_receipt(tx_hash).await? else {
        return Ok(None);
    };
    if receipt.transaction_hash != tx_hash {
        return Ok(None);
    }

    let (Some(included_block_number), Some(included_block_hash)) =
        (receipt.block_number, receipt.block_hash)
    else {
        return Ok(None);
    };
    let (finalized_block_number, finalized_block_hash) =
        match finalized_ancestor(&provider, included_block_number, included_block_hash).await {
            Ok(Some(head)) => head,
            Ok(None) | Err(Error::FinalizedAncestryPending) => return Ok(None),
            Err(error) => return Err(error),
        };

    Ok(Some(FinalizedTransactionReceipt {
        receipt,
        included_block_number,
        included_block_hash,
        finalized_block_number,
        finalized_block_hash,
    }))
}

#[derive(Clone)]
pub struct EthApi {
    contracts: Contracts,
    chain_id: u64,
    public_key: Address,
    wallet: EthereumWallet,
    url: Url,
    ws_max_retry: Option<u32>,
    ws_retry_interval: Option<Duration>,
    // Simple nonce management queries the pending nonce for every send. Serialize
    // submissions across all EthApi clones so concurrent calls cannot reuse it.
    submission_lock: Arc<Mutex<()>>,
}

impl EthApi {
    pub async fn new(
        url: &str,
        message_queue_address: &str,
        private_key: Option<&str>,
        max_fee_per_gas: Option<u128>,
        max_priority_fee_per_gas: Option<u128>,
    ) -> Result<EthApi, Error> {
        Self::new_with_retries(
            url,
            message_queue_address,
            private_key,
            None,
            None,
            max_fee_per_gas,
            max_priority_fee_per_gas,
        )
        .await
    }

    pub async fn new_with_retries(
        url: &str,
        message_queue_address: &str,
        private_key: Option<&str>,
        ws_max_retry: Option<u32>,
        ws_retry_interval: Option<Duration>,
        max_fee_per_gas: Option<u128>,
        max_priority_fee_per_gas: Option<u128>,
    ) -> Result<EthApi, Error> {
        let signer = match private_key {
            Some(private_key) => {
                let pk: B256 =
                    B256::from(U256::from_str(private_key).map_err(|_| Error::WrongPrivateKey)?);
                PrivateKeySigner::from_bytes(&pk).map_err(|_| Error::WrongPrivateKey)?
            }
            None => PrivateKeySigner::random(),
        };

        let public_key = signer.address();

        let wallet = EthereumWallet::from(signer);

        let message_queue_address: Address = message_queue_address
            .parse()
            .map_err(|_| Error::WrongAddress)?;

        let url = Url::parse(url).map_err(|_| Error::WrongNodeUrl)?;

        let mut ws = WsConnect::new(url.clone());

        if let Some(ws_max_retry) = ws_max_retry {
            ws = ws.with_max_retries(ws_max_retry)
        }

        if let Some(ws_retry_interval) = ws_retry_interval {
            ws = ws.with_retry_interval(ws_retry_interval)
        }

        let provider: ProviderType = ProviderBuilder::new()
            .disable_recommended_fillers()
            .with_gas_estimation()
            .with_blob_gas_estimation()
            .with_simple_nonce_management()
            .fetch_chain_id()
            .wallet(wallet.clone())
            .connect_client(connect_ws(ws).await?);

        let chain_id = provider.get_chain_id().await?;
        let contracts = Contracts::new(
            provider,
            message_queue_address.into_array(),
            max_fee_per_gas,
            max_priority_fee_per_gas,
        )?;

        Ok(EthApi {
            contracts,
            chain_id,
            public_key,
            url,
            wallet,
            ws_max_retry,
            ws_retry_interval,
            submission_lock: submission_lock(chain_id, public_key),
        })
    }

    pub async fn reconnect(&self) -> Result<EthApi, Error> {
        let mut ws = WsConnect::new(self.url.clone());
        if let Some(ws_max_retry) = self.ws_max_retry {
            ws = ws.with_max_retries(ws_max_retry);
        }
        if let Some(ws_retry_interval) = self.ws_retry_interval {
            ws = ws.with_retry_interval(ws_retry_interval);
        }
        let provider: ProviderType = ProviderBuilder::new()
            .disable_recommended_fillers()
            .with_gas_estimation()
            .with_blob_gas_estimation()
            .with_simple_nonce_management()
            .fetch_chain_id()
            .wallet(self.wallet.clone())
            .connect_client(connect_ws(ws).await?);

        let contracts = Contracts::new(
            provider,
            self.contracts.message_queue_instance.address().0 .0,
            Some(self.contracts.max_fee_per_gas),
            Some(self.contracts.max_priority_fee_per_gas),
        )?;

        Ok(EthApi {
            contracts,
            chain_id: self.chain_id,
            public_key: self.public_key,
            url: self.url.clone(),
            wallet: self.wallet.clone(),
            ws_max_retry: self.ws_max_retry,
            ws_retry_interval: self.ws_retry_interval,
            submission_lock: self.submission_lock.clone(),
        })
    }

    // TODO: Don't expose provider here.
    pub fn raw_provider(&self) -> &ProviderType {
        &self.contracts.provider
    }
    pub fn sender_address(&self) -> H160 {
        H160::from_slice(self.public_key.as_slice())
    }

    pub fn message_queue_address(&self) -> H160 {
        H160::from_slice(self.contracts.message_queue_instance.address().as_slice())
    }

    pub async fn reserve_submission(&self) -> SubmissionGuard {
        SubmissionGuard {
            _guard: self.submission_lock.clone().lock_owned().await,
            sender: self.public_key,
            lock: self.submission_lock.clone(),
        }
    }

    pub async fn verified_finalized_view(&self) -> Result<FinalizedView, Error> {
        verified_finalized_view(self.raw_provider()).await
    }

    pub async fn verified_finalized_view_with_archive(
        &self,
        directory: &std::path::Path,
        expected_genesis: B256,
    ) -> AnyResult<FinalizedView> {
        let (header, genesis) = finality_pin(self.raw_provider()).await?;
        ensure!(
            genesis == expected_genesis,
            "finalized archive network/genesis mismatch"
        );
        register_archive(directory, genesis)?;
        let head = (header.number, header.hash);
        ensure!(
            finalized_ancestor_at(self.raw_provider(), head.0, head.1, &header, genesis)
                .await?
                .is_some(),
            "durable finalized generation conflicts with the fresh canonical head; HOLD"
        );
        Ok(FinalizedView {
            header,
            genesis,
            blocks: BTreeMap::from([head]),
        })
    }

    pub async fn enable_finality_archive(
        &self,
        directory: &std::path::Path,
        expected_genesis: B256,
    ) -> AnyResult<()> {
        enable_finality_archive(self.raw_provider(), directory, expected_genesis).await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn prepare_content_message(
        &self,
        guard: &SubmissionGuard,
        block_number: u32,
        total_leaves: u32,
        leaf_index: u32,
        nonce: [u8; 32],
        sender: [u8; 32],
        receiver: [u8; 20],
        payload: Vec<u8>,
        proof: Vec<[u8; 32]>,
    ) -> Result<PreparedContentMessage, Error> {
        use alloy::eips::Encodable2718;
        if guard.sender != self.public_key || !Arc::ptr_eq(&guard.lock, &self.submission_lock) {
            return Err(Error::InvalidPreparedTransaction(
                "submission guard belongs to another account".into(),
            ));
        }
        let account_nonce = self
            .raw_provider()
            .get_transaction_count(self.public_key)
            .pending()
            .await?;
        let chain_id = self.raw_provider().get_chain_id().await?;
        if chain_id != self.chain_id {
            return Err(Error::InvalidPreparedTransaction(
                "configured chain changed before signing".into(),
            ));
        }
        let request = self
            .contracts
            .content_message_request(
                U256::from(block_number),
                U256::from(total_leaves),
                U256::from(leaf_index),
                U256::from_be_bytes(nonce),
                B256::from(sender),
                Address::from(receiver),
                Bytes::from(payload),
                proof.into_iter().map(B256::from).collect(),
                account_nonce,
                self.public_key,
            )
            .await?
            .with_chain_id(chain_id);
        let filled = self.raw_provider().fill(request).await?;
        let raw_transaction = filled
            .as_envelope()
            .ok_or_else(|| Error::InvalidPreparedTransaction("transaction was not signed".into()))?
            .encoded_2718();
        let prepared = PreparedContentMessage {
            chain_id,
            contract: Address::from(self.message_queue_address().0),
            sender: self.public_key,
            nonce: account_nonce,
            hash: alloy::primitives::keccak256(&raw_transaction),
            raw_transaction,
        };
        prepared_content_message_identity(&prepared)
            .map_err(|error| Error::InvalidPreparedTransaction(error.to_string()))?;
        Ok(prepared)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn validate_prepared_content_message(
        &self,
        prepared: &PreparedContentMessage,
        block_number: u32,
        total_leaves: u32,
        leaf_index: u32,
        nonce: [u8; 32],
        sender: [u8; 32],
        receiver: [u8; 20],
        payload: Vec<u8>,
        proof: Vec<[u8; 32]>,
    ) -> AnyResult<()> {
        use alloy::{
            consensus::{Transaction, TxEnvelope},
            eips::Decodable2718,
            sol_types::SolCall,
        };
        prepared_content_message_identity(prepared)?;
        ensure!(
            prepared.contract == Address::from(self.message_queue_address().0)
                && prepared.sender == self.public_key
                && prepared.chain_id == self.chain_id,
            "prepared transaction belongs to another lane or signer"
        );
        let mut raw = prepared.raw_transaction.as_slice();
        let transaction = TxEnvelope::decode_2718(&mut raw)?;
        let calldata = IMessageQueue::processMessageCall {
            blockNumber: U256::from(block_number),
            totalLeaves: U256::from(total_leaves),
            leafIndex: U256::from(leaf_index),
            message: VaraMessage {
                nonce: U256::from_be_bytes(nonce),
                source: B256::from(sender),
                destination: Address::from(receiver),
                payload: Bytes::from(payload),
            },
            proof: proof.into_iter().map(B256::from).collect(),
        }
        .abi_encode();
        ensure!(
            transaction.input().as_ref() == calldata.as_slice()
                && transaction.value() == U256::ZERO,
            "prepared transaction message, proof or value changed"
        );
        Ok(())
    }

    pub async fn broadcast_prepared_content_message(
        &self,
        guard: &SubmissionGuard,
        prepared: &PreparedContentMessage,
    ) -> Result<TxHash, Error> {
        if guard.sender != self.public_key
            || !Arc::ptr_eq(&guard.lock, &self.submission_lock)
            || prepared.sender != self.public_key
            || prepared.contract != Address::from(self.message_queue_address().0)
            || self.raw_provider().get_chain_id().await? != prepared.chain_id
        {
            return Err(Error::InvalidPreparedTransaction(
                "prepared transaction belongs to another account or chain".into(),
            ));
        }
        prepared_content_message_identity(prepared)
            .map_err(|error| Error::InvalidPreparedTransaction(error.to_string()))?;
        let submitted = self
            .raw_provider()
            .send_raw_transaction(&prepared.raw_transaction)
            .await?;
        if *submitted.tx_hash() != prepared.hash {
            return Err(Error::InvalidPreparedTransaction(
                "RPC returned a different signed hash".into(),
            ));
        }
        Ok(prepared.hash)
    }

    /// Returns the maximum block number that can be submitted as part of a
    /// merkle-root submission into MessageQueue contract.
    pub async fn max_block_number(&self) -> Result<u32, Error> {
        self.contracts.max_block_number().await
    }

    /// Returns the maximum block distance allowed between `max_block_number`
    /// and the block number being submitted as part of a merkle-root submission
    /// into MessageQueue contract.
    pub async fn max_block_distance(&self) -> Result<u32, Error> {
        self.contracts.max_block_distance().await
    }

    /// Returns the delay (in seconds) requires before merkle-root submitted
    /// by an admin can be used.
    pub async fn process_admin_message_delay(&self) -> Result<u64, Error> {
        self.contracts.process_admin_message_delay().await
    }

    /// Returns the delay (in seconds) requires before merkle-root submitted
    /// by a pauser can be used.
    pub async fn process_pauser_message_delay(&self) -> Result<u64, Error> {
        self.contracts.process_pauser_message_delay().await
    }

    /// Returns the delay (in seconds) requires before merkle-root submitted
    /// by an arbitrary user can be used.
    pub async fn process_user_message_delay(&self) -> Result<u64, Error> {
        self.contracts.process_user_message_delay().await
    }

    pub async fn get_approx_balance(&self) -> Result<f64, Error> {
        self.contracts.get_approx_balance(self.public_key).await
    }

    pub async fn provide_merkle_root(
        &self,
        block_number: u32,
        merkle_root: [u8; 32],
        proof: Vec<u8>,
    ) -> Result<PendingTransactionBuilder<Ethereum>, Error> {
        let _submission_guard = self.submission_lock.lock().await;
        self.contracts
            .provide_merkle_root(
                U256::from(block_number),
                B256::from(merkle_root),
                Bytes::from(proof),
            )
            .await
    }

    pub async fn send_challenge_root(&self) -> Result<TxHash, Error> {
        let _submission_guard = self.submission_lock.lock().await;
        self.contracts.challenge_root().await
    }

    pub async fn get_tx_status(&self, tx_hash: TxHash) -> Result<TxStatus, Error> {
        self.contracts.get_tx_status(tx_hash).await
    }

    /// Returns a receipt only after its included block is canonical and finalized.
    pub async fn get_finalized_receipt(
        &self,
        tx_hash: TxHash,
    ) -> Result<Option<FinalizedTransactionReceipt>, Error> {
        finalized_transaction_receipt(self.raw_provider(), tx_hash).await
    }
    /// Revalidate a saved historical snapshot against the same finalized ancestry as receipts.
    /// An unfinished or unavailable proof is an error, not a conflicting block.
    pub async fn is_finalized_block(&self, number: u64, hash: H256) -> AnyResult<bool> {
        Ok(
            finalized_ancestor(self.raw_provider(), number, B256::from(hash.0))
                .await?
                .is_some(),
        )
    }

    pub async fn read_finalized_merkle_root(
        &self,
        gear_block: u32,
    ) -> Result<Option<[u8; 32]>, Error> {
        self.contracts
            .read_merkle_root(U256::from(gear_block), BlockNumberOrTag::Finalized)
            .await
    }
    /// Reads a queue root at the exact Ethereum block recorded by finalized evidence.
    /// Returns None if the supplied number/hash is not canonical and finalized.
    pub async fn read_finalized_merkle_root_at(
        &self,
        gear_block: u32,
        ethereum_block_number: u64,
        ethereum_block_hash: B256,
    ) -> Result<Option<[u8; 32]>, Error> {
        self.contracts
            .read_merkle_root_at_finalized_block(
                U256::from(gear_block),
                ethereum_block_number,
                ethereum_block_hash,
            )
            .await
    }

    pub async fn read_chainhead_merkle_root(
        &self,
        gear_block: u32,
    ) -> Result<Option<[u8; 32]>, Error> {
        self.contracts
            .read_merkle_root(U256::from(gear_block), BlockNumberOrTag::Latest)
            .await
    }

    pub async fn fetch_merkle_roots_in_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<(MerkleRootEntry, Option<u64>)>, Error> {
        self.contracts.fetch_merkle_roots_in_range(from, to).await
    }

    pub async fn get_block_timestamp(&self, block: u64) -> Result<u64, Error> {
        Ok(self
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Number(block))
            .await
            .map_err(Error::ErrorInHTTPTransport)?
            .ok_or(Error::ErrorFetchingBlock)?
            .header
            .timestamp)
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.contracts.block_number().await
    }

    pub async fn finalized_block_number(&self) -> AnyResult<u64> {
        Ok(self::finalized_block(self.raw_provider())
            .await?
            .header
            .number)
    }

    pub async fn latest_block_number(&self) -> AnyResult<u64> {
        Ok(self::latest_block(self.raw_provider()).await?.header.number)
    }

    pub async fn safe_block_number(&self) -> AnyResult<u64> {
        Ok(self::safe_block(self.raw_provider()).await?.header.number)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn provide_content_message(
        &self,
        _submission_guard: &SubmissionGuard,
        block_number: u32,
        total_leaves: u32,
        leaf_index: u32,
        nonce: [u8; 32],
        sender: [u8; 32],
        receiver: [u8; 20],
        payload: Vec<u8>,
        proof: Vec<[u8; 32]>,
        account_nonce: Option<u64>,
    ) -> Result<(TxHash, u64), ContentMessageSubmissionError> {
        let account_nonce = match account_nonce {
            Some(nonce) => nonce,
            None => self
                .raw_provider()
                .get_transaction_count(self.public_key)
                .pending()
                .await
                .map_err(|error| ContentMessageSubmissionError {
                    error: error.into(),
                    nonce: None,
                })?,
        };

        self.contracts
            .provide_content_message(
                U256::from(block_number),
                U256::from(total_leaves),
                U256::from(leaf_index),
                U256::from_be_bytes(nonce),
                B256::from(sender),
                Address::from(receiver),
                Bytes::from(payload),
                proof.into_iter().map(B256::from).collect(),
                account_nonce,
            )
            .await
            .map(|tx_hash| (tx_hash, account_nonce))
            .map_err(|error| {
                // Only pin the nonce after eth_sendRawTransaction was attempted.
                // Pre-send failures must re-query pending nonce on retry because
                // another serialized submission may use this nonce meanwhile.
                let nonce =
                    matches!(error, Error::ErrorSendingTransaction(_)).then_some(account_nonce);
                ContentMessageSubmissionError { error, nonce }
            })
    }

    /// Reads the relayer nonce and bridge processed bit from one canonical finalized block.
    /// None means the finalized block could not be confirmed canonical; callers must hold.
    pub async fn get_finalized_submission_snapshot(
        &self,
        pinned_nonce: u64,
        message_nonce: [u8; 32],
    ) -> Result<Option<FinalizedSubmissionSnapshot>, Error> {
        self.contracts
            .finalized_submission_snapshot(
                self.public_key,
                pinned_nonce,
                U256::from_be_bytes(message_nonce),
            )
            .await
    }

    pub async fn is_message_processed(&self, nonce: [u8; 32]) -> Result<bool, Error> {
        self.contracts
            .is_message_processed(U256::from_be_bytes(nonce))
            .await
    }

    pub async fn subscribe_logs(
        &self,
    ) -> Result<Subscription<RpcLog>, RpcError<TransportErrorKind>> {
        let filter = Filter::new()
            .address(*self.contracts.message_queue_instance.address())
            .event_signature(IMessageQueue::MerkleRoot::SIGNATURE_HASH);

        self.raw_provider().clone().subscribe_logs(&filter).await
    }

    pub async fn subscribe_blocks(
        &self,
    ) -> Result<Subscription<Header>, RpcError<TransportErrorKind>> {
        self.raw_provider().clone().subscribe_blocks().await
    }
}

impl Contracts {
    pub fn new(
        provider: ProviderType,
        message_queue_address: [u8; 20],
        max_fee_per_gas: Option<u128>,
        max_priority_fee_per_gas: Option<u128>,
    ) -> Result<Self, Error> {
        let message_queue_address = Address::from(message_queue_address);
        let message_queue_instance = IMessageQueue::new(message_queue_address, provider.clone());

        Ok(Contracts {
            provider,
            message_queue_instance,
            max_fee_per_gas: max_fee_per_gas.unwrap_or(MAX_FEE_PER_GAS),
            max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or(MAX_PRIORITY_FEE_PER_GAS),
        })
    }

    pub async fn max_block_number(&self) -> Result<u32, Error> {
        self.message_queue_instance
            .maxBlockNumber()
            .call()
            .await
            .map(|num| num.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn max_block_distance(&self) -> Result<u32, Error> {
        self.message_queue_instance
            .MAX_BLOCK_DISTANCE()
            .call()
            .await
            .map(|num| num.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn process_admin_message_delay(&self) -> Result<u64, Error> {
        self.message_queue_instance
            .PROCESS_ADMIN_MESSAGE_DELAY()
            .call()
            .await
            .map(|delay| delay.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn process_pauser_message_delay(&self) -> Result<u64, Error> {
        self.message_queue_instance
            .PROCESS_PAUSER_MESSAGE_DELAY()
            .call()
            .await
            .map(|delay| delay.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn process_user_message_delay(&self) -> Result<u64, Error> {
        self.message_queue_instance
            .PROCESS_USER_MESSAGE_DELAY()
            .call()
            .await
            .map(|delay| delay.to())
            .map_err(Error::ErrorDuringContractExecution)
    }

    pub async fn get_approx_balance(&self, address: Address) -> Result<f64, Error> {
        let balance = self.provider.get_balance(address).latest().await?;
        let balance: f64 = balance.into();
        Ok(balance / 1_000_000_000_000_000_000.0)
    }

    pub async fn provide_merkle_root(
        &self,
        block_number: U256,
        merkle_root: B256,
        proof: Bytes,
    ) -> Result<PendingTransactionBuilder<Ethereum>, Error> {
        match self
            .message_queue_instance
            .submitMerkleRoot(block_number, merkle_root, proof.clone())
            .estimate_gas()
            .await
        {
            Ok(gas_used) => {
                log::info!("Gas used: {gas_used}");
                match self
                    .message_queue_instance
                    .submitMerkleRoot(block_number, merkle_root, proof.clone())
                    .send()
                    .await
                {
                    Ok(pending_tx) => Ok(pending_tx),
                    Err(e) => {
                        log::error!("Sending error: {e:?}");
                        if let Some(e) =
                            e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                        {
                            return Err(Error::MessageQueue(e));
                        }

                        Err(Error::ErrorSendingTransaction(e))
                    }
                }
            }

            Err(e) => {
                if let Some(e) =
                    e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(e));
                }

                Err(Error::ErrorDuringContractExecution(e))
            }
        }
    }

    pub async fn challenge_root(&self) -> Result<TxHash, Error> {
        match self
            .message_queue_instance
            .challengeRoot()
            .estimate_gas()
            .await
        {
            Ok(gas_used) => {
                log::info!("Gas used: {gas_used}");
                match self.message_queue_instance.challengeRoot().send().await {
                    Ok(pending_tx) => Ok(*pending_tx.tx_hash()),
                    Err(e) => {
                        log::error!("Sending error: {e:?}");
                        if let Some(e) =
                            e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                        {
                            return Err(Error::MessageQueue(e));
                        }

                        Err(Error::ErrorSendingTransaction(e))
                    }
                }
            }

            Err(e) => {
                if let Some(e) =
                    e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(e));
                }

                Err(Error::ErrorDuringContractExecution(e))
            }
        }
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.provider.get_block_number().await.map_err(|e| e.into())
    }

    pub async fn fetch_merkle_roots(
        &self,
        depth: u64,
    ) -> Result<Vec<(MerkleRootEntry, Option<u64>)>, Error> {
        let current_block: u64 = self.provider.get_block_number().await?;

        self.fetch_merkle_roots_in_range(
            current_block.checked_sub(depth).unwrap_or_default(),
            current_block,
        )
        .await
    }

    pub async fn fetch_merkle_roots_in_range(
        &self,
        from: u64,
        to: u64,
    ) -> Result<Vec<(MerkleRootEntry, Option<u64>)>, Error> {
        let filter = Filter::new()
            .address(*self.message_queue_instance.address())
            .event_signature(IMessageQueue::MerkleRoot::SIGNATURE_HASH)
            .from_block(from)
            .to_block(to);

        let event: Event<ProviderType, MerkleRoot, Ethereum> =
            Event::new(self.provider.clone(), filter);

        let logs = event.query().await.map_err(Error::ErrorQueryingEvent)?;

        Ok(logs
            .iter()
            .map(|(event, log)| {
                (
                    MerkleRootEntry {
                        block_number: event.blockNumber.to(),
                        merkle_root: event.merkleRoot.0.into(),
                    },
                    log.block_number,
                )
            })
            .collect())
    }

    #[allow(clippy::too_many_arguments)]
    async fn content_message_request(
        &self,
        block_number: U256,
        total_leaves: U256,
        leaf_index: U256,
        nonce: U256,
        source: B256,
        destination: Address,
        payload: Bytes,
        proof: Vec<B256>,
        account_nonce: u64,
        sender: Address,
    ) -> Result<alloy::rpc::types::TransactionRequest, Error> {
        log::trace!(
            "provide_content_message: block_number = {block_number}, total_leaves = {total_leaves}, leaf_index = {leaf_index}, nonce = {nonce}, source = {source}, destination = {destination}, payload = {payload}, proof = {proof:?}",
        );

        let call = self
            .message_queue_instance
            .processMessage(
                block_number,
                total_leaves,
                leaf_index,
                VaraMessage {
                    nonce,
                    source,
                    destination,
                    payload,
                },
                proof,
            )
            .from(sender);

        let gas_estimated = match call.estimate_gas().await {
            Ok(gas_estimated) => gas_estimated,
            Err(e) => {
                if let Some(e) =
                    e.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(e));
                }

                return Err(Error::ErrorDuringContractExecution(e));
            }
        };

        let (max_priority_fee_per_gas, gas_price) = tokio::join!(
            self.provider.get_max_priority_fee_per_gas(),
            self.provider.get_gas_price()
        );
        log::trace!("max_priority_fee_per_gas_chain = {max_priority_fee_per_gas:?}, gas_price_chain = {gas_price:?}");

        let max_fee_per_gas = gas_price
            .map(|gas_price| {
                if gas_price < MAX_FEE_PER_GAS {
                    MAX_FEE_PER_GAS
                } else {
                    gas_price
                }
            })
            .unwrap_or(MAX_FEE_PER_GAS);

        let max_priority_fee_per_gas = max_priority_fee_per_gas
            .map(|max_priority_fee_per_gas| {
                if max_priority_fee_per_gas < MAX_PRIORITY_FEE_PER_GAS {
                    MAX_PRIORITY_FEE_PER_GAS
                } else {
                    max_priority_fee_per_gas
                }
            })
            .unwrap_or(MAX_PRIORITY_FEE_PER_GAS);

        let call = call
            .gas(gas_estimated)
            .nonce(account_nonce)
            .max_fee_per_gas(max_fee_per_gas)
            .max_priority_fee_per_gas(max_priority_fee_per_gas);

        let request = call.as_ref();
        log::trace!(
            "new max_priority_fee_per_gas = {max_priority_fee_per_gas:?}, new max_fee_per_gas = {max_fee_per_gas:?}, gas_estimated = {gas_estimated}, gas_price = {:?}, max_fee_per_gas = {:?}, max_priority_fee_per_gas = {:?}, gas_limit = {:?}",
            request.gas_price(),
            request.max_fee_per_gas(),
            request.max_priority_fee_per_gas(),
            request.gas_limit(),
        );

        Ok(call.into_transaction_request())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn provide_content_message(
        &self,
        block_number: U256,
        total_leaves: U256,
        leaf_index: U256,
        nonce: U256,
        source: B256,
        destination: Address,
        payload: Bytes,
        proof: Vec<B256>,
        account_nonce: u64,
    ) -> Result<TxHash, Error> {
        use alloy::providers::WalletProvider;
        let request = self
            .content_message_request(
                block_number,
                total_leaves,
                leaf_index,
                nonce,
                source,
                destination,
                payload,
                proof,
                account_nonce,
                self.provider.default_signer_address(),
            )
            .await?;
        match self.provider.send_transaction(request).await {
            Ok(pending) => Ok(*pending.tx_hash()),
            Err(error) => {
                let error = alloy::contract::Error::from(error);
                if let Some(rejection) =
                    error.as_decoded_interface_error::<IMessageQueue::IMessageQueueErrors>()
                {
                    return Err(Error::MessageQueue(rejection));
                }
                Err(Error::ErrorSendingTransaction(error))
            }
        }
    }

    pub async fn read_merkle_root(
        &self,
        block: U256,
        block_tag: BlockNumberOrTag,
    ) -> Result<Option<[u8; 32]>, Error> {
        let at = if block_tag == BlockNumberOrTag::Finalized {
            verified_finalized_view(&self.provider).await?.block_id()
        } else {
            BlockId::Number(block_tag)
        };
        self.read_merkle_root_at(block, at).await
    }

    async fn read_merkle_root_at(
        &self,
        block: U256,
        block_id: BlockId,
    ) -> Result<Option<[u8; 32]>, Error> {
        let root = self
            .message_queue_instance
            .getMerkleRoot(block)
            .block(block_id)
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?
            .0;

        Ok((root != [0; 32]).then_some(root))
    }
    pub async fn read_merkle_root_at_finalized_block(
        &self,
        block: U256,
        ethereum_block_number: u64,
        ethereum_block_hash: B256,
    ) -> Result<Option<[u8; 32]>, Error> {
        match finalized_ancestor(&self.provider, ethereum_block_number, ethereum_block_hash).await {
            Ok(Some(_)) => {
                self.read_merkle_root_at(block, BlockId::hash_canonical(ethereum_block_hash))
                    .await
            }
            Ok(None) | Err(Error::FinalizedAncestryPending) => Ok(None),
            Err(error) => Err(error),
        }
    }

    async fn finalized_submission_snapshot(
        &self,
        account: Address,
        pinned_nonce: u64,
        message_nonce: U256,
    ) -> Result<Option<FinalizedSubmissionSnapshot>, Error> {
        let view = match verified_finalized_view(&self.provider).await {
            Ok(view) => view,
            Err(Error::FinalizedAncestryPending | Error::FinalizedAncestryConflict) => {
                return Ok(None)
            }
            Err(error) => return Err(error),
        };
        let block_number = view.block_number();
        let block_hash = view.block_hash();
        let block_id = view.block_id();

        let account_nonce = self
            .provider
            .get_transaction_count(account)
            .block_id(block_id)
            .await?;
        let message_processed = self
            .is_message_processed_at(message_nonce, block_id)
            .await?;

        Ok(Some(FinalizedSubmissionSnapshot {
            block_number,
            block_hash,
            pinned_nonce_consumed: account_nonce > pinned_nonce,
            message_processed,
        }))
    }

    pub async fn is_message_processed(&self, nonce: U256) -> Result<bool, Error> {
        let view = verified_finalized_view(&self.provider).await?;
        self.is_message_processed_at(nonce, view.block_id()).await
    }

    async fn is_message_processed_at(&self, nonce: U256, block_id: BlockId) -> Result<bool, Error> {
        let processed = self
            .message_queue_instance
            .isProcessed(nonce)
            .block(block_id)
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?;

        Ok(processed)
    }

    pub async fn get_tx_status(&self, tx_hash: TxHash) -> Result<TxStatus, Error> {
        let Some(receipt) = finalized_transaction_receipt(&self.provider, tx_hash).await? else {
            return Ok(TxStatus::Pending);
        };

        Ok(if receipt.receipt.status() {
            TxStatus::Finalized
        } else {
            TxStatus::Failed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static ANCESTRY_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    struct ClearOwnedChains<const N: usize>([B256; N]);
    impl<const N: usize> Drop for ClearOwnedChains<N> {
        fn drop(&mut self) {
            if let Some(state) = FINALIZED_ANCESTRY.get() {
                let mut cache = state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                for genesis in self.0 {
                    cache.remove(&genesis);
                }
            }
        }
    }

    fn header_chain(seed: u8, height: u64) -> Vec<Block> {
        let mut parent = B256::ZERO;
        (0..=height)
            .map(|number| {
                let mut block: Block = Block::default();
                block.header.inner.number = number;
                block.header.inner.parent_hash = parent;
                block.header.inner.extra_data = Bytes::from(vec![seed]);
                block.header.hash = block.header.inner.hash_slow();
                parent = block.header.hash;
                block
            })
            .collect()
    }

    fn mocked_ancestry_api(asserter: alloy::transports::mock::Asserter) -> AnyResult<EthApi> {
        let signer = PrivateKeySigner::from_bytes(&B256::from([62; 32]))?;
        let public_key = signer.address();
        let wallet = EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .with_gas_estimation()
            .with_blob_gas_estimation()
            .with_simple_nonce_management()
            .fetch_chain_id()
            .wallet(wallet.clone())
            .connect_mocked_client(asserter);
        Ok(EthApi {
            contracts: Contracts::new(provider, [9; 20], None, None)?,
            chain_id: 560048,
            public_key,
            wallet,
            url: Url::parse("ws://127.0.0.1:1")?,
            ws_max_retry: None,
            ws_retry_interval: None,
            submission_lock: submission_lock(560048, public_key),
        })
    }

    #[test]
    fn submission_locks_are_shared_per_chain_and_public_key() {
        let first_key = Address::from([1; 20]);
        let second_key = Address::from([2; 20]);

        let first = submission_lock(1, first_key);
        let same_account = submission_lock(1, first_key);
        let different_key = submission_lock(1, second_key);
        let different_chain = submission_lock(2, first_key);

        assert!(Arc::ptr_eq(&first, &same_account));
        assert!(!Arc::ptr_eq(&first, &different_key));
        assert!(!Arc::ptr_eq(&first, &different_chain));
    }

    #[tokio::test]
    async fn saved_finalized_anchor_retries_pending_and_rejects_only_proven_conflict(
    ) -> AnyResult<()> {
        let _ancestry_test_guard = ANCESTRY_TEST_LOCK.lock().await;
        use alloy::transports::mock::Asserter;

        let chain = header_chain(61, 4);
        let hash = |number: u64| chain[number as usize].header.hash;
        let block = |number: u64| chain[number as usize].clone();
        let asserter = Asserter::new();
        let api = mocked_ancestry_api(asserter.clone())?;
        let public_key = api.public_key;
        asserter.push_success(&block(2));
        asserter.push_success(&block(0));
        assert!(api.is_finalized_block(2, hash(2).0.into()).await?);

        for (case, target, responses) in [
            ("missing finalized head", 2, vec![None]),
            ("not yet finalized", 3, vec![Some(block(2)), Some(block(0))]),
            ("missing genesis", 2, vec![Some(block(2)), None]),
            ("invalid genesis", 2, vec![Some(block(2)), Some(block(1))]),
            ("regressed head", 1, vec![Some(block(1)), Some(block(0))]),
            (
                "missing ancestry",
                1,
                vec![Some(block(2)), Some(block(0)), None, None],
            ),
            (
                "invalid hash response",
                1,
                vec![Some(block(2)), Some(block(0)), None, Some(block(1))],
            ),
        ] {
            for response in responses {
                asserter.push_success(&response);
            }
            assert!(
                api.is_finalized_block(target, hash(target).0.into())
                    .await
                    .is_err(),
                "{case} must retry, not report a conflicting finalized anchor"
            );
            assert!(asserter.read_q().is_empty(), "unconsumed {case} response");
        }

        asserter.push_success(&block(2));
        asserter.push_success(&block(0));
        asserter.push_success(&block(2));
        assert!(api.is_finalized_block(1, hash(1).0.into()).await?);
        asserter.push_success(&block(2));
        asserter.push_success(&block(0));
        assert!(!api.is_finalized_block(1, H256::from([63; 32])).await?);

        let mut fork = block(3);
        fork.header.inner.parent_hash = B256::from([64; 32]);
        fork.header.hash = fork.header.inner.hash_slow();
        asserter.push_success(&fork);
        asserter.push_success(&block(0));
        asserter.push_success(&fork);
        assert!(!api.is_finalized_block(2, hash(2).0.into()).await?);

        // A prefix above the previous trusted head cannot authorize even the new head.
        asserter.push_success(&block(4));
        asserter.push_success(&block(0));
        asserter.push_success(&block(4));
        asserter.push_success(&Option::<Block>::None);
        asserter.push_success(&Option::<Block>::None);
        assert!(api.is_finalized_block(4, hash(4).0.into()).await.is_err());
        asserter.push_success(&block(4));
        asserter.push_success(&block(0));
        asserter.push_success(&Option::<Block>::None);
        asserter.push_success(&Option::<Block>::None);
        assert!(api.is_finalized_block(4, hash(4).0.into()).await.is_err());
        let tx_hash = B256::from([65; 32]);
        let receipt: TransactionReceipt = serde_json::from_value(serde_json::json!({
            "type": "0x2", "status": "0x1", "cumulativeGasUsed": "0x0",
            "logs": [], "logsBloom": format!("0x{}", "00".repeat(256)),
            "transactionHash": tx_hash, "transactionIndex": "0x0",
            "blockHash": hash(4), "blockNumber": "0x4",
            "gasUsed": "0x0", "effectiveGasPrice": "0x0",
            "from": public_key, "to": Address::from([9; 20]), "contractAddress": null,
        }))?;
        for boundary in ["receipt", "status", "root", "snapshot", "processed"] {
            if matches!(boundary, "receipt" | "status") {
                asserter.push_success(&receipt);
            }
            asserter.push_success(&block(4));
            asserter.push_success(&block(0));
            asserter.push_success(&Option::<Block>::None);
            asserter.push_success(&Option::<Block>::None);
            match boundary {
                "receipt" => assert!(api.get_finalized_receipt(tx_hash).await?.is_none()),
                "status" => assert!(matches!(
                    api.get_tx_status(tx_hash).await?,
                    TxStatus::Pending
                )),
                "root" => assert_eq!(
                    api.read_finalized_merkle_root_at(5, 4, hash(4)).await?,
                    None
                ),
                "snapshot" => assert!(api
                    .get_finalized_submission_snapshot(9, [8; 32])
                    .await?
                    .is_none()),
                "processed" => assert!(matches!(
                    api.is_message_processed([8; 32]).await,
                    Err(Error::FinalizedAncestryPending)
                )),
                _ => unreachable!(),
            }
        }
        asserter.push_success(&block(4));
        asserter.push_success(&block(0));
        asserter.push_success(&block(3));
        assert!(api.is_finalized_block(4, hash(4).0.into()).await?);
        asserter.push_success(&receipt);
        asserter.push_success(&block(4));
        asserter.push_success(&block(0));
        let included = api
            .get_finalized_receipt(tx_hash)
            .await?
            .context("verified receipt missing")?;
        assert_eq!(included.included_block_hash, hash(4));
        assert_eq!(included.finalized_block_hash, hash(4));
        asserter.push_success(&receipt);
        asserter.push_success(&block(4));
        asserter.push_success(&block(0));
        assert!(matches!(
            api.get_tx_status(tx_hash).await?,
            TxStatus::Finalized
        ));
        asserter.push_success(&block(4));
        asserter.push_success(&block(0));
        asserter.push_success(&B256::from([66; 32]));
        assert_eq!(
            api.read_finalized_merkle_root_at(5, 4, hash(4)).await?,
            Some([66; 32])
        );
        assert!(asserter.read_q().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn pending_finalized_head_fences_survive_cache_capacity_pressure() -> AnyResult<()> {
        use alloy::transports::mock::Asserter;
        let _ancestry_test_guard = ANCESTRY_TEST_LOCK.lock().await;
        let chains: BTreeMap<_, _> = (80..=88)
            .map(|seed| (seed, header_chain(seed, 3)))
            .collect();
        let hash = |seed: u8, number: u64| chains[&seed][number as usize].header.hash;
        let block = |seed: u8, number: u64| chains[&seed][number as usize].clone();
        let _owned_chains: ClearOwnedChains<9> =
            ClearOwnedChains(std::array::from_fn(|index| hash(80 + index as u8, 0)));
        let mut fenced = Vec::with_capacity(8);
        for seed in 80..88 {
            let asserter = Asserter::new();
            let api = mocked_ancestry_api(asserter.clone())?;
            asserter.push_success(&block(seed, 1));
            asserter.push_success(&block(seed, 0));
            assert!(api.is_finalized_block(1, hash(seed, 1).0.into()).await?);
            asserter.push_success(&block(seed, 3));
            asserter.push_success(&block(seed, 0));
            asserter.push_success(&block(seed, 3));
            asserter.push_success(&Option::<Block>::None);
            asserter.push_success(&Option::<Block>::None);
            assert!(api
                .is_finalized_block(3, hash(seed, 3).0.into())
                .await
                .is_err());
            assert!(asserter.read_q().is_empty());
            fenced.push((seed, asserter, api));
        }
        let newcomer = Asserter::new();
        let newcomer_api = mocked_ancestry_api(newcomer.clone())?;
        newcomer.push_success(&block(88, 1));
        newcomer.push_success(&block(88, 0));
        let admission = newcomer_api
            .is_finalized_block(1, hash(88, 1).0.into())
            .await;
        // Exercise every real API: arbitrary HashMap eviction must not hide the lost fence.
        for (seed, asserter, api) in &fenced {
            asserter.push_success(&block(*seed, 3));
            asserter.push_success(&block(*seed, 0));
            asserter.push_success(&Option::<Block>::None);
            asserter.push_success(&Option::<Block>::None);
            let result = api.is_finalized_block(3, hash(*seed, 3).0.into()).await;
            assert!(
                matches!(&result, Err(error) if matches!(error.downcast_ref::<Error>(), Some(Error::FinalizedAncestryPending))),
                "capacity pressure must not authorize an unbridged finalized head for chain {seed}: {result:?}"
            );
            assert!(asserter.read_q().is_empty());
        }
        assert!(
            matches!(&admission, Err(error) if matches!(error.downcast_ref::<Error>(), Some(Error::FinalizedAncestryPending))),
            "all fenced slots must hold new-chain admission: {admission:?}"
        );
        let (seed, asserter, api) = &fenced[0];
        asserter.push_success(&block(*seed, 3));
        asserter.push_success(&block(*seed, 0));
        asserter.push_success(&block(*seed, 2));
        assert!(api.is_finalized_block(3, hash(*seed, 3).0.into()).await?);
        newcomer.push_success(&block(88, 1));
        newcomer.push_success(&block(88, 0));
        assert!(
            newcomer_api
                .is_finalized_block(1, hash(88, 1).0.into())
                .await?
        );
        assert!(newcomer.read_q().is_empty() && asserter.read_q().is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn finalized_ancestry_rejects_a_numbered_block_from_another_backend() -> AnyResult<()> {
        let _ancestry_test_guard = ANCESTRY_TEST_LOCK.lock().await;
        use alloy::transports::mock::Asserter;
        let chain = header_chain(21, 103);
        let a_head = chain[102].header.hash;
        let a_inclusion = chain[100].header.hash;
        let b_inclusion = B256::from([24; 32]);
        let block = |number, seed: u8, parent_hash| {
            let mut block: Block = Block::default();
            block.header.inner.number = number;
            block.header.inner.parent_hash = parent_hash;
            block.header.inner.extra_data = Bytes::from(vec![seed]);
            block.header.hash = block.header.inner.hash_slow();
            block
        };
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let genesis = chain[0].clone();
        let head = chain[102].clone();
        let parent = chain[101].clone();
        asserter.push_success(&head);
        asserter.push_success(&genesis);
        asserter.push_success(&head);
        asserter.push_success(&parent);
        // A numbered block on backend B at height 100 cannot override A's
        // parent-linked canonical inclusion at that height.
        assert_eq!(finalized_ancestor(&provider, 100, b_inclusion).await?, None);
        asserter.push_success(&head);
        asserter.push_success(&genesis);
        assert_eq!(
            finalized_ancestor(&provider, 100, a_inclusion).await?,
            Some((102, a_head))
        );

        let fork = block(103, 26, B256::from([27; 32]));
        asserter.push_success(&fork);
        asserter.push_success(&genesis);
        asserter.push_success(&fork);
        assert_eq!(
            finalized_ancestor(&provider, 100, a_inclusion).await?,
            None,
            "a newer finalized tag on another branch must not reuse cached ancestry"
        );
        let extension = block(103, 28, a_head);
        asserter.push_success(&extension);
        asserter.push_success(&genesis);
        asserter.push_success(&extension);
        assert_eq!(
            finalized_ancestor(&provider, 100, a_inclusion).await?,
            Some((103, extension.header.hash))
        );

        let other_head = block(102, 31, B256::ZERO);
        let other_genesis = block(0, 32, B256::ZERO);
        asserter.push_success(&other_head);
        asserter.push_success(&other_genesis);
        assert_eq!(
            finalized_ancestor(&provider, 102, other_head.header.hash).await?,
            Some((102, other_head.header.hash)),
            "another genesis must not inherit Hoodi cache"
        );
        assert!(asserter.read_q().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn historical_inclusion_progresses_without_an_age_cutoff() -> AnyResult<()> {
        let _ancestry_test_guard = ANCESTRY_TEST_LOCK.lock().await;
        use alloy::transports::mock::Asserter;
        let chain = header_chain(41, 258);
        let hash = |number: u64| chain[number as usize].header.hash;
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let head = chain[258].clone();
        let genesis = chain[0].clone();
        asserter.push_success(&head);
        asserter.push_success(&genesis);
        for number in (1..=258).rev() {
            asserter.push_success(&chain[number as usize]);
        }
        assert_eq!(
            finalized_ancestor(&provider, 0, hash(0)).await?,
            Some((258, hash(258)))
        );
        // Subsequent historical receipts share the already verified lineage.
        asserter.push_success(&head);
        asserter.push_success(&genesis);
        assert_eq!(
            finalized_ancestor(&provider, 1, hash(1)).await?,
            Some((258, hash(258)))
        );
        assert!(asserter.read_q().is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn rate_limited_ancestry_resumes_only_verified_parent_links() -> AnyResult<()> {
        let _ancestry_test_guard = ANCESTRY_TEST_LOCK.lock().await;
        use alloy::transports::mock::Asserter;
        let chain = header_chain(55, 21);
        let hash = |number: u64| chain[number as usize].header.hash;
        let block = |number: u64| chain[number as usize].clone();
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&block(20));
        asserter.push_success(&block(0));
        for number in (3..=20).rev() {
            asserter.push_success(&block(number));
        }
        asserter.push_success(&Option::<Block>::None);
        asserter.push_success(&block(1));
        asserter.push_success(&Option::<Block>::None);
        assert!(matches!(
            finalized_ancestor(&provider, 0, hash(0)).await,
            Err(Error::FinalizedAncestryPending)
        ));
        asserter.push_success(&block(21));
        asserter.push_success(&block(0));
        asserter.push_success(&block(21));
        asserter.push_success(&block(2));
        asserter.push_success(&block(1));
        assert_eq!(
            finalized_ancestor(&provider, 0, hash(0)).await?,
            Some((21, hash(21)))
        );
        assert!(asserter.read_q().is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn interrupted_new_head_link_holds_until_old_head_matches() -> AnyResult<()> {
        let _ancestry_test_guard = ANCESTRY_TEST_LOCK.lock().await;
        use alloy::transports::mock::Asserter;
        let chain = header_chain(59, 41);
        let hash = |number: u64| chain[number as usize].header.hash;
        let block = |number: u64| chain[number as usize].clone();
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&block(20));
        asserter.push_success(&block(0));
        assert_eq!(
            finalized_ancestor(&provider, 20, hash(20)).await?,
            Some((20, hash(20)))
        );
        asserter.push_success(&block(40));
        asserter.push_success(&block(0));
        for number in (25..=40).rev() {
            asserter.push_success(&block(number));
        }
        asserter.push_success(&Option::<Block>::None);
        for number in (21..=23).rev() {
            asserter.push_success(&block(number));
        }
        asserter.push_success(&Option::<Block>::None);
        assert!(matches!(
            finalized_ancestor(&provider, 40, hash(40)).await,
            Err(Error::FinalizedAncestryPending)
        ));
        // Even a target equal to the new head must wait for the old head link.
        asserter.push_success(&block(41));
        asserter.push_success(&block(0));
        asserter.push_success(&block(41));
        for number in (21..=24).rev() {
            asserter.push_success(&block(number));
        }
        assert_eq!(
            finalized_ancestor(&provider, 40, hash(40)).await?,
            Some((41, hash(41)))
        );
        assert!(asserter.read_q().is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn nonce_processed_and_root_reads_use_one_finalized_hash_or_hold() -> AnyResult<()> {
        let _ancestry_test_guard = ANCESTRY_TEST_LOCK.lock().await;
        use serde_json::json;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = Url::parse(&format!("http://{}", listener.local_addr()?))?;
        let chain = header_chain(51, 100);
        let hash = chain[100].header.hash;
        let head = serde_json::to_value(&chain[100])?;
        let genesis = serde_json::to_value(&chain[0])?;
        let pinned = json!({"blockHash": format!("{hash:#x}"), "requireCanonical": true});
        let pinned_for_server = pinned.clone();
        let root = [67u8; 32];
        let expected_root = format!("{:#x}", B256::from(root));
        let server = tokio::spawn(async move {
            let mut observed = Vec::new();
            for index in 0..13 {
                let (mut socket, _) = listener.accept().await?;
                let mut buffer = Vec::new();
                let request = loop {
                    let mut chunk = [0u8; 4096];
                    let count = socket.read(&mut chunk).await?;
                    ensure!(count > 0, "RPC request ended before its JSON body");
                    buffer.extend_from_slice(&chunk[..count]);
                    let Some(end) = buffer.windows(4).position(|window| window == b"\r\n\r\n")
                    else {
                        continue;
                    };
                    let headers = std::str::from_utf8(&buffer[..end])?;
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .and_then(|(_, value)| value.trim().parse::<usize>().ok())
                        })
                        .context("RPC request omitted Content-Length")?;
                    if buffer.len() >= end + 4 + length {
                        break serde_json::from_slice::<Value>(&buffer[end + 4..end + 4 + length])?;
                    }
                };
                let method = request["method"].as_str().context("missing RPC method")?;
                let is_pinned = request["params"][1] == pinned_for_server;
                let result = match method {
                    "eth_getBlockByNumber" => {
                        if request["params"][0] == "0x0" {
                            genesis.clone()
                        } else {
                            head.clone()
                        }
                    }
                    "eth_getTransactionCount" if index == 12 && is_pinned => {
                        let response = json!({"jsonrpc":"2.0", "id":request["id"],
                            "error":{"code":-32602,"message":"hash-pinned reads unsupported"}})
                        .to_string();
                        socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await?;
                        observed.push(request);
                        continue;
                    }
                    "eth_getTransactionCount" => json!(if is_pinned { "0x7" } else { "0x8" }),
                    "eth_call" if index == 3 || index == 6 => json!(if is_pinned {
                        format!("0x{:064x}", 1u8)
                    } else {
                        format!("0x{:064x}", 0u8)
                    }),
                    "eth_call" => json!(if is_pinned {
                        expected_root.clone()
                    } else {
                        format!("{:#x}", B256::ZERO)
                    }),
                    other => bail!("unexpected RPC method {other}"),
                };
                observed.push(request.clone());
                let response =
                    json!({"jsonrpc":"2.0", "id":request["id"], "result":result}).to_string();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await?;
            }
            Ok::<_, anyhow::Error>(observed)
        });

        let signer = PrivateKeySigner::from_bytes(&B256::from([11; 32]))?;
        let account = signer.address();
        let provider = ProviderBuilder::new()
            .disable_recommended_fillers()
            .with_gas_estimation()
            .with_blob_gas_estimation()
            .with_simple_nonce_management()
            .fetch_chain_id()
            .wallet(EthereumWallet::from(signer))
            .connect_http(url);
        let contracts = Contracts::new(provider, [9; 20], None, None)?;
        let snapshot = contracts
            .finalized_submission_snapshot(account, 7, U256::from(3))
            .await?
            .context("finalized snapshot missing")?;
        assert!(!snapshot.pinned_nonce_consumed && snapshot.message_processed);
        assert!(
            contracts.is_message_processed(U256::from(3)).await?,
            "an already-processed revert may succeed only at a hash-pinned finalized block"
        );
        assert_eq!(
            contracts
                .read_merkle_root_at_finalized_block(U256::from(5), 100, hash)
                .await?,
            Some(root)
        );
        assert!(
            contracts
                .finalized_submission_snapshot(account, 7, U256::from(3))
                .await
                .is_err(),
            "unsupported hash-pinned nonce lookup must hold rather than fall back to block number"
        );
        let requests = server.await??;
        for index in [2, 3, 6, 9, 12] {
            assert_eq!(
                requests[index]["params"][1], pinned,
                "unbound state read #{index}"
            );
        }
        Ok(())
    }
    #[tokio::test]
    async fn every_unfinished_finalized_generation_fences_state_only_acceptance() -> AnyResult<()> {
        use alloy::transports::mock::Asserter;
        let _serial = ANCESTRY_TEST_LOCK.lock().await;
        let chain = header_chain(103, 6);
        let _owned_chain = ClearOwnedChains([chain[0].header.hash]);
        let mut fork = chain.clone();
        fork[4].header.inner.extra_data = vec![0xaa].into();
        fork[4].header.hash = fork[4].header.inner.hash_slow();
        for number in 5..=6 {
            fork[number].header.inner.parent_hash = fork[number - 1].header.hash;
            fork[number].header.hash = fork[number].header.inner.hash_slow();
        }
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let api = mocked_ancestry_api(asserter.clone())?;
        asserter.push_success(&chain[2]);
        asserter.push_success(&chain[0]);
        verified_finalized_view(&provider).await?;
        asserter.push_success(&chain[4]);
        asserter.push_success(&chain[0]);
        asserter.push_success(&chain[4]);
        asserter.push_success(&Option::<Block>::None);
        asserter.push_success(&Option::<Block>::None);
        assert!(matches!(
            verified_finalized_view(&provider).await,
            Err(Error::FinalizedAncestryPending)
        ));
        asserter.push_success(&fork[6]);
        asserter.push_success(&chain[0]);
        asserter.push_success(&fork[6]);
        asserter.push_success(&Option::<Block>::None);
        asserter.push_success(&Option::<Block>::None);
        assert!(matches!(
            verified_finalized_view(&provider).await,
            Err(Error::FinalizedAncestryPending)
        ));
        // This branch reaches the original height-2 anchor, but conflicts with
        // the unfinished height-4 generation. A Processed bit cannot bypass it.
        asserter.push_success(&fork[6]);
        asserter.push_success(&chain[0]);
        asserter.push_success(&fork[5]);
        asserter.push_success(&fork[4]);
        asserter.push_success(&fork[3]);
        assert!(matches!(
            api.is_message_processed([9; 32]).await,
            Err(Error::FinalizedAncestryConflict)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn durable_history_revalidates_tenfold_age_and_records_and_retains_restart_fences(
    ) -> AnyResult<()> {
        use alloy::transports::mock::Asserter;
        let _serial = ANCESTRY_TEST_LOCK.lock().await;
        let height = 131_850;
        let chain = header_chain(101, height + 2);
        let genesis = chain[0].header.hash;
        let directory = std::env::temp_dir().join(format!(
            "beefy-finality-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        struct Cleanup {
            directory: std::path::PathBuf,
            genesis: B256,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                if let Some(cache) = FINALIZED_ANCESTRY.get() {
                    cache.lock().unwrap().remove(&self.genesis);
                }
                if let Some(archives) = FINALITY_ARCHIVES.get() {
                    archives.lock().unwrap().remove(&self.genesis);
                }
                let _ = std::fs::remove_dir_all(&self.directory);
            }
        }
        let _cleanup = Cleanup {
            directory: directory.clone(),
            genesis,
        };
        register_archive(&directory, genesis)?;
        {
            let archive = archive_for(genesis).unwrap();
            let mut top = height;
            while top > 0 {
                let bottom = top.saturating_sub(511).max(1);
                let headers: Vec<_> = (bottom..=top)
                    .rev()
                    .map(|number| chain[number as usize].header.inner.clone())
                    .collect();
                archive.save(&headers)?;
                top = bottom - 1;
            }
        }
        let targets: BTreeMap<_, _> = (0..8_430)
            .map(|index| (index * 15, chain[(index * 15) as usize].header.hash))
            .collect();
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&chain[height as usize]);
        asserter.push_success(&chain[0]);
        let mut view = verified_finalized_view(&provider).await?;
        view.verify_blocks(&provider, &targets).await?;
        for (&number, &hash) in &targets {
            assert!(view.contains(number, hash));
        }
        assert!(
            asserter.read_q().is_empty(),
            "locally checked proof material must not re-fetch historical receipt anchors"
        );

        // Corrupt hint material must be reconstructed from canonical RLP, never
        // promoted through the successful sparse target cache or discarded.
        let prefix = format!("headers-{height}-");
        let damaged = std::fs::read_dir(&directory)?
            .find_map(|entry| {
                let entry = entry.ok()?;
                entry
                    .file_name()
                    .to_str()?
                    .starts_with(&prefix)
                    .then(|| entry.path())
            })
            .context("retained head chunk")?;
        std::fs::write(&damaged, b"corrupt")?;
        asserter.push_success(&chain[height as usize]);
        asserter.push_success(&chain[0]);
        let mut repaired = verified_finalized_view(&provider).await?;
        for number in ((height - 511)..=height).rev() {
            asserter.push_success(&chain[number as usize]);
        }
        repaired
            .verify_blocks(
                &provider,
                &BTreeMap::from([(height - 512, chain[(height - 512) as usize].header.hash)]),
            )
            .await?;
        assert!(asserter.read_q().is_empty());
        assert_eq!(
            std::fs::read(&damaged)?,
            b"corrupt",
            "invalid hint evidence must remain intact"
        );

        // The next generation is witnessed before an interrupted two-header gap.
        asserter.push_success(&chain[(height + 2) as usize]);
        asserter.push_success(&chain[0]);
        asserter.push_success(&chain[(height + 2) as usize]);
        asserter.push_success(&Option::<Block>::None);
        asserter.push_success(&Option::<Block>::None);
        assert!(matches!(
            verified_finalized_view(&provider).await,
            Err(Error::FinalizedAncestryPending)
        ));
        FINALIZED_ANCESTRY
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .remove(&genesis);
        FINALITY_ARCHIVES
            .get()
            .unwrap()
            .lock()
            .unwrap()
            .remove(&genesis);
        register_archive(&directory, genesis)?;
        asserter.push_success(&chain[(height + 2) as usize]);
        asserter.push_success(&chain[0]);
        asserter.push_success(&chain[(height + 1) as usize]);
        let mut restarted = verified_finalized_view(&provider).await?;
        restarted.verify_blocks(&provider, &targets).await?;
        assert!(asserter.read_q().is_empty(), "restart must reuse only authenticated partial chunks and bridge the exact original tip");

        // A corrupt durable fence is not permission to trust a fresh generation.
        std::fs::write(directory.join("fence.json"), b"corrupt")?;
        asserter.push_success(&chain[(height + 2) as usize]);
        asserter.push_success(&chain[0]);
        assert!(matches!(
            verified_finalized_view(&provider).await,
            Err(Error::FinalizedAncestryPending)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn interrupted_interior_frontier_survives_the_recent_ancestor_bound() -> AnyResult<()> {
        use alloy::transports::mock::Asserter;
        let _serial = ANCESTRY_TEST_LOCK.lock().await;
        let height = MAX_KNOWN_ANCESTORS as u64 + 1_000;
        let newer = height + MAX_KNOWN_ANCESTORS as u64 + 1_000;
        let chain = header_chain(102, newer);
        let _owned_chain = ClearOwnedChains([chain[0].header.hash]);
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&chain[height as usize]);
        asserter.push_success(&chain[0]);
        for number in (1..=height).rev() {
            asserter.push_success(&chain[number as usize]);
        }
        assert!(finalized_ancestor(&provider, 0, chain[0].header.hash)
            .await?
            .is_some());
        let target = height - 500;
        let frontier = target + 100;
        asserter.push_success(&chain[height as usize]);
        asserter.push_success(&chain[0]);
        // The large preceding descent evicted this interior range. Stop halfway.
        for number in ((frontier + 1)..=height).rev() {
            asserter.push_success(&chain[number as usize]);
        }
        for _ in 0..33 {
            asserter.push_success(&Option::<Block>::None);
        }
        assert!(matches!(
            finalized_ancestor(&provider, target, chain[target as usize].header.hash).await,
            Err(Error::FinalizedAncestryPending)
        ));
        // Abort cancels unused speculative hints; their queued mock replies are
        // not responses to the next independently pinned view.
        asserter.write_q().clear();
        // Prove a much newer generation: its recent entries evict the interior
        // range, but must not evict the independently retained unfinished frontier.
        asserter.push_success(&chain[newer as usize]);
        asserter.push_success(&chain[0]);
        for number in ((height + 1)..=newer).rev() {
            asserter.push_success(&chain[number as usize]);
        }
        assert!(
            finalized_ancestor(&provider, newer, chain[newer as usize].header.hash)
                .await?
                .is_some()
        );
        // Start exactly at the saved verified parent. Rewalking from the head
        // would encounter these valid but wrong-height replies and cannot pass.
        asserter.push_success(&chain[newer as usize]);
        asserter.push_success(&chain[0]);
        for number in ((target + 1)..=frontier).rev() {
            asserter.push_success(&chain[number as usize]);
        }
        assert!(
            finalized_ancestor(&provider, target, chain[target as usize].header.hash)
                .await?
                .is_some()
        );
        assert!(asserter.read_q().is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn sparse_hoodi_identity_rejects_missing_or_conflicting_evidence() -> AnyResult<()> {
        use alloy::transports::mock::Asserter;
        use serde_json::json;

        // The recorded failed lookup was a type-2 transaction at nonce 0x24 with no accessList.
        let hash: B256 =
            "0x3ab31061459478dd7625369532a2cf5e4c79204d7345ccca094290be8c7c1946".parse()?;
        let from: Address = "0x312684c68309d41964d6229959e4671bb8d3878c".parse()?;
        let to: Address = "0xc040a1bf14a398df2d0bd02a24fc18e5244b0e66".parse()?;
        let sparse = json!({
            "type": "0x2", "hash": format!("{hash:#x}"), "from": format!("{from:#x}"),
            "nonce": "0x24", "to": format!("{to:#x}"), "blockNumber": "0x38be42"
        });
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        asserter.push_success(&Option::<Value>::None);
        assert_eq!(transaction_identity(&provider, hash).await?, None);
        asserter.push_success(&sparse);
        assert_eq!(
            transaction_identity(&provider, hash).await?,
            Some(TransactionIdentity {
                hash,
                from,
                nonce: 36,
                to: Some(to),
                block_number: Some(0x38be42),
            })
        );
        for (field, bad) in [
            ("hash", json!(format!("{:#x}", B256::ZERO))),
            ("from", json!("not-an-address")),
            ("nonce", json!("0x25")),
            ("nonce", json!("0x024")),
            ("to", json!(false)),
            ("blockNumber", json!("not-a-quantity")),
        ] {
            let mut malformed = sparse.clone();
            malformed[field] = bad;
            asserter.push_success(&malformed);
            let observed = transaction_identity(&provider, hash).await;
            if field == "nonce" && malformed[field] == "0x25" {
                assert_eq!(
                    observed?
                        .context("nonce changed but identity absent")?
                        .nonce,
                    37
                );
            } else {
                assert!(observed.is_err(), "{field} must be validated");
            }
        }
        for field in ["hash", "from", "nonce", "to"] {
            let mut malformed = sparse.clone();
            malformed.as_object_mut().unwrap().remove(field);
            asserter.push_success(&malformed);
            assert!(
                transaction_identity(&provider, hash).await.is_err(),
                "missing {field}"
            );
        }
        Ok(())
    }

    #[test]
    fn legacy_contract_creation_identity_keeps_null_destination_and_pending_block() -> AnyResult<()>
    {
        use serde_json::json;
        let hash = B256::from([1; 32]);
        let legacy = json!({"type": "0x0", "hash": format!("{hash:#x}"),
            "from": format!("{:#x}", Address::from([2; 20])),
            "nonce": "0x0", "to": null, "blockNumber": null});
        let identity = decode_transaction_identity(&legacy, hash)?;
        assert_eq!(identity.nonce, 0);
        assert_eq!(identity.to, None);
        assert_eq!(identity.block_number, None);
        Ok(())
    }
}
