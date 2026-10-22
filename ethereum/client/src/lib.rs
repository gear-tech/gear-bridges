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
#[cfg(test)]
mod finality_tests;
pub mod inclusion;
mod queue_deployment;
pub mod signed_transaction;
pub use queue_deployment::QueueDeployment;
use tokio::time::{timeout, Instant};

const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

async fn connect_rpc(
    url: &Url,
    ws_max_retry: Option<u32>,
    ws_retry_interval: Option<Duration>,
) -> Result<alloy::rpc::client::RpcClient, Error> {
    finality_endpoint_identity(url).map_err(|_| Error::WrongNodeUrl)?;
    let builder = alloy::rpc::client::RpcClient::builder()
        .layer(tower::util::MapFutureLayer::new(
            |future| -> alloy::transports::TransportFut<'static> { Box::pin(future) },
        ))
        .layer(tower::util::MapErrLayer::new(
            |error: tower::BoxError| match error.downcast::<TransportError>() {
                Ok(error) => *error,
                Err(error) => RpcError::Transport(TransportErrorKind::Custom(error)),
            },
        ))
        .layer(tower::timeout::TimeoutLayer::new(RPC_REQUEST_TIMEOUT));
    match url.scheme() {
        "http" | "https" => Ok(builder.http(url.clone())),
        "ws" | "wss" => {
            let mut ws = WsConnect::new(url.clone());
            if let Some(retries) = ws_max_retry {
                ws = ws.with_max_retries(retries);
            }
            if let Some(interval) = ws_retry_interval {
                ws = ws.with_retry_interval(interval);
            }
            Ok(timeout(RPC_REQUEST_TIMEOUT, builder.ws(ws))
                .await
                .map_err(TransportErrorKind::custom)??)
        }
        _ => Err(Error::WrongNodeUrl),
    }
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
pub use error::{is_transient_rpc_error, Error};

/// Holds exclusive ownership of Ethereum account submissions across retries.
pub struct SubmissionGuard {
    _guard: OwnedMutexGuard<()>,
    sender: Address,
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
    prepared_content_message_transaction(prepared).map(|(_, identity)| identity)
}

/// Validate original signed bytes once and retain the decoded envelope for typed calldata checks.
pub fn prepared_content_message_transaction(
    prepared: &PreparedContentMessage,
) -> AnyResult<(alloy::consensus::TxEnvelope, TransactionIdentity)> {
    use signed_transaction::{
        decode_signed_transaction, SignedTransactionErrors, SignedTransactionExpected,
    };
    decode_signed_transaction(
        &prepared.raw_transaction,
        SignedTransactionExpected {
            hash: prepared.hash,
            nonce: prepared.nonce,
            destination: prepared.contract,
            chain_id: Some(prepared.chain_id),
            signer: Some(prepared.sender),
        },
        SignedTransactionErrors {
            hash: "prepared transaction hash changed",
            decode: "invalid prepared signed transaction",
            trailing: None,
            identity: "prepared transaction nonce, chain or contract changed",
            signature: "invalid prepared transaction signature",
            signer: "prepared transaction signer changed",
        },
    )
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
    finality: Option<FinalityWitness>,
    message_queue_instance: IMessageQueueInstance<ProviderType, Ethereum>,
    max_fee_per_gas: u128,
    max_priority_fee_per_gas: u128,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MerkleRootEntry {
    pub block_number: u64,
    pub merkle_root: H256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MerkleRootObservation {
    pub entry: MerkleRootEntry,
    pub block_number: u64,
    pub block_hash: B256,
    pub transaction_hash: B256,
    pub transaction_index: u64,
    pub log_index: u64,
    pub removed: bool,
}

fn merkle_root_observation(
    event: &MerkleRoot,
    log: &RpcLog,
    from: u64,
    to: u64,
) -> Result<MerkleRootObservation, Error> {
    let invalid = |field: &str| {
        Error::InvalidMerkleRootObservation(format!("HOLD: invalid merkle root {field}"))
    };
    let source_block =
        u32::try_from(event.blockNumber).map_err(|_| invalid("source block exceeds u32"))?;
    let block_number = log.block_number.ok_or_else(|| invalid("block number"))?;
    if !(from..=to).contains(&block_number) {
        return Err(invalid("inclusion outside requested range"));
    }
    Ok(MerkleRootObservation {
        entry: MerkleRootEntry {
            block_number: u64::from(source_block),
            merkle_root: event.merkleRoot.0.into(),
        },
        block_number,
        block_hash: log.block_hash.ok_or_else(|| invalid("block hash"))?,
        transaction_hash: log
            .transaction_hash
            .ok_or_else(|| invalid("transaction hash"))?,
        transaction_index: log
            .transaction_index
            .ok_or_else(|| invalid("transaction index"))?,
        log_index: log.log_index.ok_or_else(|| invalid("log index"))?,
        removed: log.removed,
    })
}

fn validate_merkle_root_observations(observations: &[MerkleRootObservation]) -> Result<(), Error> {
    let mut logs = BTreeMap::new();
    let mut blocks = BTreeMap::new();
    let mut transactions = BTreeMap::new();
    let mut transaction_indices = BTreeMap::new();
    for observation in observations {
        let location = (
            observation.block_number,
            observation.block_hash,
            observation.transaction_index,
        );
        if logs
            .insert(
                (observation.block_number, observation.log_index),
                observation,
            )
            .is_some_and(|old| old != observation)
            || blocks
                .insert(observation.block_number, observation.block_hash)
                .is_some_and(|old| old != observation.block_hash)
            || transactions
                .insert(observation.transaction_hash, location)
                .is_some_and(|old| old != location)
            || transaction_indices
                .insert(
                    (observation.block_number, observation.transaction_index),
                    observation.transaction_hash,
                )
                .is_some_and(|old| old != observation.transaction_hash)
        {
            return Err(Error::InvalidMerkleRootObservation(
                "HOLD: conflicting merkle root inclusion identities".into(),
            ));
        }
    }
    Ok(())
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
    pub inclusion: inclusion::RecordedReceiptProof,
    pub checkpoint: FinalityCheckpoint,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalizedSubmissionSnapshot {
    pub block_number: u64,
    pub block_hash: B256,
    pub pinned_nonce_consumed: bool,
    pub message_processed: bool,
}

/// Execution finality is independently corroborated, not Beacon consensus verification.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FinalityCheckpoint {
    pub schema_version: u32,
    pub policy: String,
    pub chain_id: u64,
    pub genesis_hash: B256,
    pub common_header: alloy::consensus::Header,
    pub primary_finalized_header: alloy::consensus::Header,
    pub witness_finalized_header: alloy::consensus::Header,
}

#[derive(Clone, Debug)]
struct FinalityWitness {
    provider: RootProvider,
    chain_id: u64,
    genesis: B256,
}

// URL spelling cannot establish operator independence; reject obvious aliases.
fn finality_endpoint_identity(url: &Url) -> AnyResult<(&str, &str, u16, &str, Option<&str>)> {
    let host = url
        .host_str()
        .context("HOLD: Ethereum endpoint has no host")?
        .trim_end_matches('.');
    let address = host
        .strip_prefix('[')
        .and_then(|host| host.strip_suffix(']'))
        .unwrap_or(host);
    let loopback = host == "localhost"
        || address
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    let scheme = match url.scheme() {
        "https" | "wss" => "https",
        "http" | "ws" if loopback => "http",
        _ => {
            bail!("HOLD: Ethereum endpoints require HTTPS/WSS except literal loopback or localhost")
        }
    };
    Ok((
        scheme,
        if loopback { "localhost" } else { host },
        url.port_or_known_default()
            .context("HOLD: Ethereum endpoint has no port")?,
        url.path().trim_end_matches('/'),
        url.query(),
    ))
}

impl FinalityWitness {
    async fn connect(
        primary: &Url,
        witness: &str,
        chain_id: u64,
        genesis: B256,
        retries: Option<u32>,
        interval: Option<Duration>,
    ) -> AnyResult<Self> {
        let witness = Url::parse(witness)?;
        ensure!(
            finality_endpoint_identity(primary)? != finality_endpoint_identity(&witness)?,
            "HOLD: primary and witness endpoints must be independent",
        );
        Ok(Self {
            provider: RootProvider::builder()
                .connect_client(connect_rpc(&witness, retries, interval).await?),
            chain_id,
            genesis,
        })
    }
}

fn require_witness(witness: &Option<FinalityWitness>) -> Result<&FinalityWitness, Error> {
    witness.as_ref().ok_or(Error::FinalityWitnessNotConfigured)
}

#[derive(Clone)]
pub struct PollingEthApi {
    provider: RootProvider,
    finality: Option<FinalityWitness>,
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
        let provider = RootProvider::builder()
            .connect_client(connect_rpc(&url, ws_max_retry, ws_retry_interval).await?);

        Ok(Self {
            provider,
            finality: None,
            url,
            ws_max_retry,
            ws_retry_interval,
        })
    }

    pub async fn reconnect(&self) -> AnyResult<Self> {
        let mut connected =
            Self::new_with_retries(self.url.as_str(), self.ws_max_retry, self.ws_retry_interval)
                .await?;
        connected.finality = self.finality.clone();
        Ok(connected)
    }

    pub async fn with_finality_witness(
        mut self,
        witness_url: &str,
        expected_chain_id: u64,
        expected_genesis: B256,
    ) -> AnyResult<Self> {
        self.finality = Some(
            FinalityWitness::connect(
                &self.url,
                witness_url,
                expected_chain_id,
                expected_genesis,
                self.ws_max_retry,
                self.ws_retry_interval,
            )
            .await?,
        );
        Ok(self)
    }

    pub fn finality_identity(&self) -> AnyResult<(u64, B256)> {
        let witness = require_witness(&self.finality)?;
        Ok((witness.chain_id, witness.genesis))
    }

    /// Raw read-only transport. Pair historical answers at a qualified hash.
    pub fn raw_witness_provider(&self) -> AnyResult<&RootProvider> {
        Ok(&require_witness(&self.finality)?.provider)
    }
    pub async fn verified_finalized_view(&self) -> Result<FinalizedView, Error> {
        verified_finalized_view(&self.provider, require_witness(&self.finality)?).await
    }

    pub async fn enable_finality_archive(
        &self,
        directory: &std::path::Path,
        expected_genesis: B256,
    ) -> AnyResult<()> {
        ensure!(
            self.finality_identity()?.1 == expected_genesis,
            "archive approval mismatch"
        );
        enable_finality_archive(&self.provider, directory, expected_genesis).await
    }

    pub async fn is_finalized_block(&self, number: u64, hash: H256) -> AnyResult<bool> {
        Ok(finalized_ancestor(
            &self.provider,
            require_witness(&self.finality)?,
            number,
            B256::from(hash.0),
        )
        .await?)
    }

    pub async fn chain_id(&self) -> AnyResult<u64> {
        Ok(self.provider.get_chain_id().await?)
    }

    pub async fn finalized_block(&self) -> AnyResult<Block> {
        let view = self.verified_finalized_view().await?;
        let block = self
            .provider
            .get_block_by_hash(view.block_hash())
            .await?
            .context("qualified block unavailable")?;
        ensure!(
            valid_header(&block, view.block_number(), view.block_hash()),
            "qualified block changed"
        );
        view.recheck(&self.provider).await?;
        Ok(block)
    }

    /// Returns a receipt only after its included block is canonical and finalized.
    pub async fn get_finalized_receipt(
        &self,
        tx_hash: TxHash,
    ) -> AnyResult<Option<FinalizedTransactionReceipt>> {
        Ok(
            finalized_transaction_receipt(
                &self.provider,
                require_witness(&self.finality)?,
                tx_hash,
            )
            .await?,
        )
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
            .await
            .map_err(Error::ErrorQueryingEvent)?
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
            .await
            .map_err(Error::ErrorQueryingEvent)?
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
}

impl Deref for PollingEthApi {
    type Target = RootProvider;

    fn deref(&self) -> &Self::Target {
        &self.provider
    }
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
    std::sync::Mutex<HashMap<B256, Vec<Arc<finality_archive::HeaderArchive>>>>,
> = OnceLock::new();
static FINALITY_LOCKS: OnceLock<std::sync::Mutex<HashMap<B256, Weak<Mutex<()>>>>> = OnceLock::new();
const MAX_CHAINS: usize = 8;
const MAX_KNOWN_ANCESTORS: usize = 32_768;
const MAX_PROOF_DURATION: Duration = Duration::from_secs(300);
// Eight mainnet epochs of advertised lag; not a historical ancestry/proof limit.
const MAX_FINALIZED_TIP_DISTANCE: u64 = 256;

impl VerifiedAncestry {
    fn remember(&mut self, proof: impl IntoIterator<Item = (u64, B256)>) {
        self.known.extend(proof);
        while self.known.len() > MAX_KNOWN_ANCESTORS {
            self.known.pop_first();
        }
    }
}

fn archives_for(genesis: B256) -> Vec<Arc<finality_archive::HeaderArchive>> {
    FINALITY_ARCHIVES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&genesis)
        .cloned()
        .unwrap_or_default()
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
    std::fs::create_dir_all(directory)?;
    let directory = std::fs::canonicalize(directory)?;
    let mut archives = FINALITY_ARCHIVES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (chain, registered) in archives.iter() {
        if registered
            .iter()
            .any(|archive| archive.directory() == directory)
        {
            ensure!(
                *chain == genesis,
                "finality archive belongs to another chain; HOLD"
            );
            return Ok(());
        }
    }
    let archive = Arc::new(finality_archive::HeaderArchive::open(&directory)?);
    archives.entry(genesis).or_default().push(archive);
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

async fn qualified_header(
    provider: &impl Provider,
    tag: BlockNumberOrTag,
) -> Result<Header, Error> {
    let block = timeout(Duration::from_secs(10), provider.get_block_by_number(tag))
        .await
        .map_err(|_| Error::FinalizedAncestryPending)??
        .ok_or(Error::FinalizedAncestryPending)?;
    let number = match tag {
        BlockNumberOrTag::Number(number) => number,
        _ => block.header.number,
    };
    if !valid_header(&block, number, block.header.hash) {
        return Err(Error::FinalizedAncestryConflict);
    }
    Ok(block.header)
}

async fn finality_pin(
    provider: &impl Provider,
    witness: &FinalityWitness,
    started: Instant,
) -> Result<(Header, FinalityCheckpoint), Error> {
    let mut tips = Vec::with_capacity(2);
    for endpoint in [provider.root(), &witness.provider] {
        let chain_id = timeout(Duration::from_secs(10), endpoint.get_chain_id())
            .await
            .map_err(|_| Error::FinalizedAncestryPending)??;
        let genesis = qualified_header(endpoint, BlockNumberOrTag::Number(0)).await?;
        if chain_id != witness.chain_id || genesis.hash != witness.genesis {
            return Err(Error::FinalizedAncestryConflict);
        }
        let tip = qualified_header(endpoint, BlockNumberOrTag::Finalized).await?;
        if tip.number == 0 && tip.hash != genesis.hash {
            return Err(Error::FinalizedAncestryConflict);
        }
        tips.push(tip);
    }
    if tips[0].number.abs_diff(tips[1].number) > MAX_FINALIZED_TIP_DISTANCE {
        return Err(Error::FinalizedAncestryPending);
    }
    let height = tips[0].number.min(tips[1].number);
    let common = qualified_header(provider, BlockNumberOrTag::Number(height)).await?;
    let other = qualified_header(&witness.provider, BlockNumberOrTag::Number(height)).await?;
    if common.inner != other.inner || common.hash != other.hash {
        return Err(Error::FinalizedAncestryConflict);
    }
    for (endpoint, tip) in [provider.root(), &witness.provider].into_iter().zip(&tips) {
        let (number, hash, _) =
            walk_ancestors(endpoint, (tip.number, tip.hash), height, started, &[], None).await?;
        if number != height || started.elapsed() >= MAX_PROOF_DURATION {
            return Err(Error::FinalizedAncestryPending);
        }
        if hash != common.hash {
            return Err(Error::FinalizedAncestryConflict);
        }
    }
    let checkpoint = FinalityCheckpoint {
        schema_version: 1,
        policy: "independent-two-el-v1".into(),
        chain_id: witness.chain_id,
        genesis_hash: witness.genesis,
        common_header: common.inner.clone(),
        primary_finalized_header: tips[0].inner.clone(),
        witness_finalized_header: tips[1].inner.clone(),
    };
    Ok((common, checkpoint))
}

async fn walk_ancestors(
    provider: &impl Provider,
    start: (u64, B256),
    target: u64,
    started: Instant,
    archives: &[Arc<finality_archive::HeaderArchive>],
    targets: Option<&BTreeMap<u64, B256>>,
) -> Result<(u64, B256, Vec<(u64, B256)>), Error> {
    use std::collections::VecDeque;
    let (mut number, mut hash) = start;
    let mut verified = VecDeque::new();
    let mut requests = tokio::task::JoinSet::new();
    let mut ready = BTreeMap::new();
    let mut next_request = number;
    let mut local = VecDeque::new();
    let archive = archives.first();
    let mut chunk = Vec::with_capacity(archive.map_or(0, |_| 512));
    while number > target && started.elapsed() < MAX_PROOF_DURATION {
        if local.is_empty() {
            for candidate in archives {
                let stored = candidate.read(number, hash).unwrap_or_default();
                if !stored.is_empty() {
                    // A local chunk can jump past every speculative numbered
                    // hint. Cancel them before opening another bounded window.
                    requests.shutdown().await;
                    ready.clear();
                    next_request = number;
                    if !chunk.is_empty() {
                        candidate
                            .save(&chunk)
                            .map_err(|_| Error::FinalizedAncestryPending)?;
                        chunk.clear();
                    }
                }
                local = stored.into();
                if !local.is_empty() {
                    break;
                }
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
            if let Some(archive) = archive {
                chunk.push(block.header.inner.clone());
                if chunk.len() == 512 {
                    archive
                        .save(&chunk)
                        .map_err(|_| Error::FinalizedAncestryPending)?;
                    chunk.clear();
                }
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
    checkpoint: FinalityCheckpoint,
    witness: FinalityWitness,
}

impl FinalizedView {
    pub fn checkpoint(&self) -> &FinalityCheckpoint {
        &self.checkpoint
    }

    /// Recheck the exact pin, never replace staged evidence with a newer head.
    pub async fn recheck(&self, provider: &impl Provider) -> Result<(), Error> {
        for endpoint in [provider.root(), &self.witness.provider] {
            let header =
                qualified_header(endpoint, BlockNumberOrTag::Number(self.block_number())).await?;
            if header.inner != self.checkpoint.common_header || header.hash != self.block_hash() {
                return Err(Error::FinalizedAncestryConflict);
            }
        }
        Ok(())
    }

    pub async fn verify_checkpoint(
        &mut self,
        provider: &impl Provider,
        checkpoint: &FinalityCheckpoint,
    ) -> Result<(), Error> {
        if checkpoint.schema_version != 1
            || checkpoint.policy != "independent-two-el-v1"
            || checkpoint.chain_id != self.checkpoint.chain_id
            || checkpoint.genesis_hash != self.genesis
            || checkpoint.common_header.number > checkpoint.primary_finalized_header.number
            || checkpoint.common_header.number > checkpoint.witness_finalized_header.number
        {
            return Err(Error::FinalizedAncestryConflict);
        }
        self.verify_blocks(
            provider,
            &BTreeMap::from([(
                checkpoint.common_header.number,
                checkpoint.common_header.hash_slow(),
            )]),
        )
        .await
    }

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
        let started = Instant::now();
        self.recheck(provider).await?;
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
        let archives = archives_for(self.genesis);
        if let Some(expected) = targets.get(&self.block_number()) {
            if *expected != self.block_hash() {
                return Err(Error::FinalizedAncestryConflict);
            }
        }
        let (number, hash, proof) = walk_ancestors(
            provider,
            (self.block_number(), self.block_hash()),
            oldest,
            started,
            &archives,
            Some(targets),
        )
        .await?;
        if number != oldest {
            return Err(Error::FinalizedAncestryPending);
        }
        if targets.get(&number) != Some(&hash) {
            return Err(Error::FinalizedAncestryConflict);
        }
        self.recheck(provider).await?;
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

async fn verified_finalized_view(
    provider: &impl Provider,
    witness: &FinalityWitness,
) -> Result<FinalizedView, Error> {
    let started = Instant::now();
    let (header, checkpoint) = finality_pin(provider, witness, started).await?;
    let genesis = checkpoint.genesis_hash;
    let head = (header.number, header.hash);
    if finalized_ancestor_at(
        provider,
        head.0,
        head.1,
        &header,
        genesis,
        started,
        Some(&checkpoint),
    )
    .await?
    .is_none()
    {
        return Err(Error::FinalizedAncestryConflict);
    }
    let view = FinalizedView {
        header,
        genesis,
        checkpoint,
        witness: witness.clone(),
        blocks: BTreeMap::from([head]),
    };
    view.recheck(provider).await?;
    let _verification = finality_lock(genesis).lock_owned().await;
    let current = FINALIZED_ANCESTRY
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !current
        .get(&genesis)
        .is_some_and(|cache| cache.head == head && cache.required_heads.is_empty())
    {
        return Err(Error::FinalizedAncestryPending);
    }
    for archive in archives_for(genesis) {
        archive
            .complete(&view.checkpoint)
            .map_err(|_| Error::FinalizedAncestryPending)?;
    }
    Ok(view)
}

async fn finalized_ancestor(
    provider: &impl Provider,
    witness: &FinalityWitness,
    number: u64,
    hash: B256,
) -> Result<bool, Error> {
    let mut view = verified_finalized_view(provider, witness).await?;
    if number > view.block_number() {
        return Err(Error::FinalizedAncestryPending);
    }
    match view
        .verify_blocks(provider, &BTreeMap::from([(number, hash)]))
        .await
    {
        Ok(()) => Ok(true),
        Err(Error::FinalizedAncestryConflict) => Ok(false),
        Err(error) => Err(error),
    }
}

async fn finalized_ancestor_at(
    provider: &impl Provider,
    number: u64,
    hash: B256,
    header: &Header,
    genesis: B256,
    started: Instant,
    checkpoint: Option<&FinalityCheckpoint>,
) -> Result<Option<(u64, B256)>, Error> {
    let _verification = finality_lock(genesis).lock_owned().await;
    let head = (header.number, header.hash);
    if number > head.0 {
        return Err(Error::FinalizedAncestryPending);
    }
    // Every directory retains its own durable fence. Registering another proof
    // owner must not replace or bypass any previously witnessed generation.
    let archives = archives_for(genesis);
    let mut durable_required = BTreeMap::new();
    for archive in &archives {
        for (number, hash) in archive
            .witness(genesis, &header.inner, checkpoint)
            .map_err(|_| Error::FinalizedAncestryPending)?
        {
            if durable_required
                .insert(number, hash)
                .is_some_and(|old| old != hash)
            {
                return Err(Error::FinalizedAncestryConflict);
            }
        }
    }
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
            let (height, ancestor, extension) =
                walk_ancestors(provider, head, old_head.0, started, &archives, None).await?;
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
            &archives,
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
    if let Some((&oldest, _)) = durable_required.first_key_value() {
        let targets = durable_required;
        let (height, ancestor, links) =
            walk_ancestors(provider, head, oldest, started, &archives, Some(&targets)).await?;
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
        walk_ancestors(provider, start, number, started, &archives, None).await?;
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
    Ok(Some(head))
}
async fn finalized_transaction_receipt(
    provider: impl Provider,
    witness: &FinalityWitness,
    tx_hash: TxHash,
) -> Result<Option<FinalizedTransactionReceipt>, Error> {
    let mut view = verified_finalized_view(&provider, witness).await?;
    let Some(receipt) = provider.get_transaction_receipt(tx_hash).await? else {
        view.recheck(&provider).await?;
        return Ok(None);
    };
    let invalid = |error: anyhow::Error| Error::InvalidFinalityEvidence(error.to_string());
    let (included_block_number, included_block_hash) = receipt
        .block_number
        .zip(receipt.block_hash)
        .ok_or_else(|| {
            Error::InvalidFinalityEvidence("receipt inclusion coordinates missing".into())
        })?;
    if included_block_number > view.block_number() {
        view.recheck(&provider).await?;
        return Ok(None);
    }
    view.verify_blocks(
        &provider,
        &BTreeMap::from([(included_block_number, included_block_hash)]),
    )
    .await?;
    let (inclusion, first_log_index) = inclusion::prove_transaction_receipt_with_log_index(
        &provider,
        &view,
        included_block_number,
        included_block_hash,
        tx_hash,
    )
    .await
    .map_err(|error| match error.downcast::<Error>() {
        Ok(error) => error,
        Err(error) => match error.downcast::<TransportError>() {
            Ok(error) => Error::from(error),
            Err(error) => invalid(error),
        },
    })?;
    validate_receipt_locator(&receipt, &inclusion, tx_hash).map_err(invalid)?;
    if receipt
        .inner
        .logs()
        .first()
        .is_some_and(|log| log.log_index != Some(first_log_index))
    {
        return Err(Error::InvalidFinalityEvidence(
            "receipt global log index differs from included predecessors".into(),
        ));
    }
    view.recheck(&provider).await?;
    Ok(Some(FinalizedTransactionReceipt {
        receipt,
        included_block_number,
        included_block_hash,
        finalized_block_number: view.block_number(),
        finalized_block_hash: view.block_hash(),
        checkpoint: view.checkpoint,
        inclusion,
    }))
}

fn validate_receipt_locator(
    receipt: &TransactionReceipt,
    proof: &inclusion::RecordedReceiptProof,
    hash: B256,
) -> AnyResult<()> {
    use alloy::{
        consensus::{transaction::SignerRecoverable, Transaction, TxEnvelope},
        eips::Decodable2718,
    };
    ensure!(
        receipt.transaction_hash == hash
            && receipt.transaction_index == Some(proof.index)
            && receipt.block_number == Some(proof.header.number)
            && receipt.block_hash == Some(proof.header.hash_slow())
            && receipt.inner.clone().into_primitives_receipt() == proof.receipt,
        "receipt locator differs from included envelope"
    );
    let mut bytes = proof.raw_transaction.as_slice();
    let transaction = TxEnvelope::decode_2718(&mut bytes)?;
    ensure!(
        bytes.is_empty() && alloy::primitives::keccak256(&proof.raw_transaction) == hash,
        "invalid exact signed envelope"
    );
    let signer = transaction.recover_signer()?;
    ensure!(
        signer == receipt.from && transaction.to() == receipt.to,
        "receipt sender/destination differs from signed original"
    );
    let created = (transaction.to().is_none() && proof.receipt.status())
        .then(|| signer.create(transaction.nonce()));
    ensure!(
        receipt.contract_address == created,
        "receipt creation address differs from signed original"
    );
    let mut previous = None;
    for log in receipt.inner.logs() {
        ensure!(
            !log.removed
                && log.block_hash == receipt.block_hash
                && log.block_number == receipt.block_number
                && log.transaction_hash == Some(hash)
                && log.transaction_index == Some(proof.index),
            "misbound or removed receipt log"
        );
        let index = log.log_index.context("missing global log index")?;
        ensure!(
            previous.is_none_or(|previous: u64| previous.checked_add(1) == Some(index)),
            "noncontiguous receipt log indices"
        );
        previous = Some(index);
    }
    Ok(())
}

#[derive(Clone)]
pub struct EthApi {
    contracts: Contracts,
    queue_deployment_transaction: Option<B256>,
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

        let provider: ProviderType = ProviderBuilder::new()
            .disable_recommended_fillers()
            .with_gas_estimation()
            .with_blob_gas_estimation()
            .with_simple_nonce_management()
            .fetch_chain_id()
            .wallet(wallet.clone())
            .connect_client(connect_rpc(&url, ws_max_retry, ws_retry_interval).await?);

        let chain_id = provider.get_chain_id().await?;
        let contracts = Contracts::new(
            provider,
            message_queue_address.into_array(),
            max_fee_per_gas,
            max_priority_fee_per_gas,
        )?;

        Ok(EthApi {
            contracts,
            queue_deployment_transaction: None,
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
        let provider: ProviderType = ProviderBuilder::new()
            .disable_recommended_fillers()
            .with_gas_estimation()
            .with_blob_gas_estimation()
            .with_simple_nonce_management()
            .fetch_chain_id()
            .wallet(self.wallet.clone())
            .connect_client(
                connect_rpc(&self.url, self.ws_max_retry, self.ws_retry_interval).await?,
            );

        let mut contracts = Contracts::new(
            provider,
            self.contracts.message_queue_instance.address().0 .0,
            Some(self.contracts.max_fee_per_gas),
            Some(self.contracts.max_priority_fee_per_gas),
        )?;
        contracts.finality = self.contracts.finality.clone();

        Ok(EthApi {
            contracts,
            queue_deployment_transaction: self.queue_deployment_transaction,
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

    pub async fn with_finality_witness(
        mut self,
        witness_url: &str,
        expected_chain_id: u64,
        expected_genesis: B256,
    ) -> AnyResult<Self> {
        ensure!(
            self.chain_id == expected_chain_id,
            "HOLD: signer network differs from approved identity"
        );
        self.contracts.finality = Some(
            FinalityWitness::connect(
                &self.url,
                witness_url,
                expected_chain_id,
                expected_genesis,
                self.ws_max_retry,
                self.ws_retry_interval,
            )
            .await?,
        );
        Ok(self)
    }

    pub fn finality_identity(&self) -> AnyResult<(u64, B256)> {
        let witness = require_witness(&self.contracts.finality)?;
        Ok((witness.chain_id, witness.genesis))
    }

    /// Raw read-only transport. Pair historical answers at a qualified hash.
    pub fn raw_witness_provider(&self) -> AnyResult<&RootProvider> {
        Ok(&require_witness(&self.contracts.finality)?.provider)
    }
    pub fn with_queue_deployment_transaction(mut self, hash: Option<B256>) -> Self {
        self.queue_deployment_transaction = hash;
        self
    }

    pub fn queue_deployment_transaction(&self) -> Option<B256> {
        self.queue_deployment_transaction
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
        }
    }

    /// Uses per-header deadlines and a cooperative traversal bound that retains
    /// authenticated progress. Do not wrap the complete proof in an RPC timeout:
    /// cancelling it can discard a partially verified ancestry frontier.
    pub async fn verified_finalized_view(&self) -> Result<FinalizedView, Error> {
        verified_finalized_view(
            self.raw_provider(),
            require_witness(&self.contracts.finality)?,
        )
        .await
    }

    /// Attach an independently owned archive without replacing same-chain owners.
    /// Every attached durable tip is linked to the fresh finalized head before
    /// this view is returned; canonical path aliases reuse the same file lock.
    pub async fn verified_finalized_view_with_archive(
        &self,
        directory: &std::path::Path,
        expected_genesis: B256,
    ) -> AnyResult<FinalizedView> {
        self.enable_finality_archive(directory, expected_genesis)
            .await?;
        Ok(self.verified_finalized_view().await?)
    }

    pub async fn enable_finality_archive(
        &self,
        directory: &std::path::Path,
        expected_genesis: B256,
    ) -> AnyResult<()> {
        ensure!(
            self.finality_identity()?.1 == expected_genesis,
            "archive approval mismatch"
        );
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
        if guard.sender != self.public_key
            || !Arc::ptr_eq(OwnedMutexGuard::mutex(&guard._guard), &self.submission_lock)
        {
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
        use alloy::{consensus::Transaction, sol_types::SolCall};
        let (transaction, _) = prepared_content_message_transaction(prepared)?;
        ensure!(
            prepared.contract == Address::from(self.message_queue_address().0)
                && prepared.sender == self.public_key
                && prepared.chain_id == self.chain_id,
            "prepared transaction belongs to another lane or signer"
        );
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
            || !Arc::ptr_eq(OwnedMutexGuard::mutex(&guard._guard), &self.submission_lock)
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

    /// Submit a challenge once. Only a transient `ChallengeRootPreBroadcast`
    /// error permits a fresh scan and retry; all other errors require a hold.
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
        finalized_transaction_receipt(
            self.raw_provider(),
            require_witness(&self.contracts.finality)?,
            tx_hash,
        )
        .await
    }
    /// Revalidate a saved historical snapshot against the same finalized ancestry as receipts.
    /// An unfinished or unavailable proof is an error, not a conflicting block.
    pub async fn is_finalized_block(&self, number: u64, hash: H256) -> AnyResult<bool> {
        Ok(finalized_ancestor(
            self.raw_provider(),
            require_witness(&self.contracts.finality)?,
            number,
            B256::from(hash.0),
        )
        .await?)
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

    /// Read queue state at the exact authenticated finalized view, without repinning.
    pub async fn read_merkle_root_in_view(
        &self,
        gear_block: u32,
        view: &FinalizedView,
    ) -> Result<Option<[u8; 32]>, Error> {
        let witness = require_witness(&self.contracts.finality)?;
        if witness.chain_id != view.checkpoint.chain_id || witness.genesis != view.genesis {
            return Err(Error::FinalizedAncestryConflict);
        }
        let root = self
            .contracts
            .read_merkle_root_at(U256::from(gear_block), view.block_id())
            .await?;
        view.recheck(self.raw_provider()).await?;
        Ok(root)
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
    ) -> Result<Vec<MerkleRootObservation>, Error> {
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

    pub async fn get_block_timestamp_by_hash(&self, number: u64, hash: B256) -> Result<u64, Error> {
        let block = self
            .raw_provider()
            .get_block_by_hash(hash)
            .await?
            .ok_or(Error::FinalizedAncestryPending)?;
        if !valid_header(&block, number, hash) {
            return Err(Error::FinalizedAncestryPending);
        }
        Ok(block.header.timestamp)
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.contracts.block_number().await
    }

    pub async fn finalized_block_number(&self) -> AnyResult<u64> {
        Ok(self.verified_finalized_view().await?.block_number())
    }

    pub async fn latest_block_number(&self) -> AnyResult<u64> {
        Ok(self::latest_block(self.raw_provider()).await?.header.number)
    }

    pub async fn safe_block_number(&self) -> AnyResult<u64> {
        Ok(self::safe_block(self.raw_provider()).await?.header.number)
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
    /// Reauthenticate every saved bit at its original canonical finalized state pin.
    /// Latest state cannot substitute for a historical nonce disposition.
    pub async fn verify_finalized_submission_snapshot(
        &self,
        pinned_nonce: u64,
        message_nonce: [u8; 32],
        saved: &FinalizedSubmissionSnapshot,
    ) -> AnyResult<()> {
        require_witness(&self.contracts.finality)?;
        ensure!(
            saved.block_hash != B256::ZERO,
            "HOLD: saved nonce disposition has no block hash"
        );
        let mut view = self.verified_finalized_view().await?;
        view.verify_blocks(
            self.raw_provider(),
            &BTreeMap::from([(saved.block_number, saved.block_hash)]),
        )
        .await?;
        let block_id = BlockId::hash_canonical(saved.block_hash);
        let account_nonce = self
            .contracts
            .agreed_account_nonce(self.public_key, block_id)
            .await?;
        let processed = self
            .contracts
            .is_message_processed_at(U256::from_be_bytes(message_nonce), block_id)
            .await?;
        ensure!(
            saved.pinned_nonce_consumed == (account_nonce > pinned_nonce)
                && saved.message_processed == processed,
            "HOLD: saved nonce disposition differs from original finalized state"
        );
        view.recheck(self.raw_provider()).await?;
        Ok(())
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
            finality: None,
            message_queue_instance,
            max_fee_per_gas: max_fee_per_gas.unwrap_or(MAX_FEE_PER_GAS),
            max_priority_fee_per_gas: max_priority_fee_per_gas.unwrap_or(MAX_PRIORITY_FEE_PER_GAS),
        })
    }

    async fn scheduling_value<C: alloy::sol_types::SolCall>(&self, call: C) -> Result<U256, Error> {
        let view =
            verified_finalized_view(&self.provider, require_witness(&self.finality)?).await?;
        let request = alloy::rpc::types::TransactionRequest::default()
            .to(*self.message_queue_instance.address())
            .input(Bytes::from(call.abi_encode()).into());
        let primary = self
            .provider
            .call(request.clone())
            .block(view.block_id())
            .await?;
        let witness = require_witness(&self.finality)?
            .provider
            .call(request)
            .block(view.block_id())
            .await?;
        if primary != witness || primary.len() != 32 {
            return Err(Error::FinalizedAncestryConflict);
        }
        view.recheck(&self.provider).await?;
        Ok(U256::from_be_slice(&primary))
    }

    pub async fn max_block_number(&self) -> Result<u32, Error> {
        self.scheduling_value(IMessageQueue::maxBlockNumberCall {})
            .await?
            .try_into()
            .map_err(|_| Error::InvalidFinalityEvidence("max block exceeds u32".into()))
    }

    pub async fn max_block_distance(&self) -> Result<u32, Error> {
        self.scheduling_value(IMessageQueue::MAX_BLOCK_DISTANCECall {})
            .await?
            .try_into()
            .map_err(|_| Error::InvalidFinalityEvidence("max distance exceeds u32".into()))
    }

    pub async fn process_admin_message_delay(&self) -> Result<u64, Error> {
        self.scheduling_value(IMessageQueue::PROCESS_ADMIN_MESSAGE_DELAYCall {})
            .await?
            .try_into()
            .map_err(|_| Error::InvalidFinalityEvidence("admin delay exceeds u64".into()))
    }

    pub async fn process_pauser_message_delay(&self) -> Result<u64, Error> {
        self.scheduling_value(IMessageQueue::PROCESS_PAUSER_MESSAGE_DELAYCall {})
            .await?
            .try_into()
            .map_err(|_| Error::InvalidFinalityEvidence("pauser delay exceeds u64".into()))
    }

    pub async fn process_user_message_delay(&self) -> Result<u64, Error> {
        self.scheduling_value(IMessageQueue::PROCESS_USER_MESSAGE_DELAYCall {})
            .await?
            .try_into()
            .map_err(|_| Error::InvalidFinalityEvidence("user delay exceeds u64".into()))
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

    /// Estimation failures are typed as `ChallengeRootPreBroadcast` (or a decoded
    /// permanent `MessageQueue` error); send failures never authorize a retry.
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

                Err(Error::ChallengeRootPreBroadcast(e))
            }
        }
    }

    pub async fn block_number(&self) -> Result<u64, Error> {
        self.provider.get_block_number().await.map_err(|e| e.into())
    }

    pub async fn fetch_merkle_roots(
        &self,
        depth: u64,
    ) -> Result<Vec<MerkleRootObservation>, Error> {
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
    ) -> Result<Vec<MerkleRootObservation>, Error> {
        let mut observations = Vec::new();
        let mut page_from = from;
        while page_from <= to {
            // Inclusive ranges stay within providers' 100,000-block log-query limit.
            let page_to = page_from.saturating_add(99_999).min(to);
            let filter = Filter::new()
                .address(*self.message_queue_instance.address())
                .event_signature(IMessageQueue::MerkleRoot::SIGNATURE_HASH)
                .from_block(page_from)
                .to_block(page_to);
            let event: Event<ProviderType, MerkleRoot, Ethereum> =
                Event::new(self.provider.clone(), filter);
            for (event, log) in event.query().await.map_err(Error::ErrorQueryingEvent)? {
                observations.push(merkle_root_observation(&event, &log, page_from, page_to)?);
            }
            if page_to == to {
                break;
            }
            page_from = page_to + 1;
        }
        observations.sort_unstable_by_key(|observation| {
            (
                observation.block_number,
                observation.transaction_index,
                observation.log_index,
            )
        });
        validate_merkle_root_observations(&observations)?;
        observations.dedup();
        Ok(observations)
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

        // The existing GasFiller estimates market fees before the original is signed.
        Ok(call
            .gas(gas_estimated)
            .nonce(account_nonce)
            .into_transaction_request())
    }

    pub async fn read_merkle_root(
        &self,
        block: U256,
        block_tag: BlockNumberOrTag,
    ) -> Result<Option<[u8; 32]>, Error> {
        if block_tag == BlockNumberOrTag::Finalized {
            let view =
                verified_finalized_view(&self.provider, require_witness(&self.finality)?).await?;
            let root = self.read_merkle_root_at(block, view.block_id()).await?;
            view.recheck(&self.provider).await?;
            Ok(root)
        } else {
            let root = self
                .message_queue_instance
                .getMerkleRoot(block)
                .block(BlockId::Number(block_tag))
                .call()
                .await
                .map_err(Error::ErrorDuringContractExecution)?
                .0;
            Ok((root != [0; 32]).then_some(root))
        }
    }

    async fn read_merkle_root_at(
        &self,
        block: U256,
        block_id: BlockId,
    ) -> Result<Option<[u8; 32]>, Error> {
        let witness = require_witness(&self.finality)?;
        let root = self
            .message_queue_instance
            .getMerkleRoot(block)
            .block(block_id)
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?
            .0;
        let other = IMessageQueue::new(*self.message_queue_instance.address(), &witness.provider)
            .getMerkleRoot(block)
            .block(block_id)
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?
            .0;
        if root != other {
            return Err(Error::FinalizedAncestryConflict);
        }

        Ok((root != [0; 32]).then_some(root))
    }
    pub async fn read_merkle_root_at_finalized_block(
        &self,
        block: U256,
        ethereum_block_number: u64,
        ethereum_block_hash: B256,
    ) -> Result<Option<[u8; 32]>, Error> {
        let mut view =
            verified_finalized_view(&self.provider, require_witness(&self.finality)?).await?;
        if ethereum_block_number > view.block_number() {
            return Ok(None);
        }
        view.verify_blocks(
            &self.provider,
            &BTreeMap::from([(ethereum_block_number, ethereum_block_hash)]),
        )
        .await?;
        let root = self
            .read_merkle_root_at(block, BlockId::hash_canonical(ethereum_block_hash))
            .await?;
        view.recheck(&self.provider).await?;
        Ok(root)
    }

    async fn finalized_submission_snapshot(
        &self,
        account: Address,
        pinned_nonce: u64,
        message_nonce: U256,
    ) -> Result<Option<FinalizedSubmissionSnapshot>, Error> {
        let view =
            verified_finalized_view(&self.provider, require_witness(&self.finality)?).await?;
        let block_number = view.block_number();
        let block_hash = view.block_hash();
        let block_id = view.block_id();

        let account_nonce = self.agreed_account_nonce(account, block_id).await?;
        let message_processed = self
            .is_message_processed_at(message_nonce, block_id)
            .await?;
        view.recheck(&self.provider).await?;

        Ok(Some(FinalizedSubmissionSnapshot {
            block_number,
            block_hash,
            pinned_nonce_consumed: account_nonce > pinned_nonce,
            message_processed,
        }))
    }

    pub async fn is_message_processed(&self, nonce: U256) -> Result<bool, Error> {
        let view =
            verified_finalized_view(&self.provider, require_witness(&self.finality)?).await?;
        let processed = self.is_message_processed_at(nonce, view.block_id()).await?;
        view.recheck(&self.provider).await?;
        Ok(processed)
    }

    async fn is_message_processed_at(&self, nonce: U256, block_id: BlockId) -> Result<bool, Error> {
        let witness = require_witness(&self.finality)?;
        let processed = self
            .message_queue_instance
            .isProcessed(nonce)
            .block(block_id)
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?;
        let other = IMessageQueue::new(*self.message_queue_instance.address(), &witness.provider)
            .isProcessed(nonce)
            .block(block_id)
            .call()
            .await
            .map_err(Error::ErrorDuringContractExecution)?;
        if processed != other {
            return Err(Error::FinalizedAncestryConflict);
        }

        Ok(processed)
    }

    async fn agreed_account_nonce(&self, account: Address, block: BlockId) -> Result<u64, Error> {
        let witness = require_witness(&self.finality)?;
        let nonce = self
            .provider
            .get_transaction_count(account)
            .block_id(block)
            .await?;
        let other = witness
            .provider
            .get_transaction_count(account)
            .block_id(block)
            .await?;
        if nonce != other {
            return Err(Error::FinalizedAncestryConflict);
        }
        Ok(nonce)
    }

    pub async fn get_tx_status(&self, tx_hash: TxHash) -> Result<TxStatus, Error> {
        let Some(receipt) = finalized_transaction_receipt(
            &self.provider,
            require_witness(&self.finality)?,
            tx_hash,
        )
        .await?
        else {
            return Ok(TxStatus::Pending);
        };

        Ok(if receipt.inclusion.receipt.status() {
            TxStatus::Finalized
        } else {
            TxStatus::Failed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unconfigured_finality_never_reports_an_absent_original_pending() -> AnyResult<()> {
        let api = mocked_ancestry_api(alloy::transports::mock::Asserter::new())?;
        for error in [
            api.get_finalized_receipt(B256::ZERO).await.unwrap_err(),
            api.get_tx_status(B256::ZERO).await.unwrap_err(),
            api.verified_finalized_view().await.unwrap_err(),
        ] {
            assert_eq!(
                error.to_string(),
                "HOLD: independent Ethereum finality witness is not configured"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn root_scan_retains_both_sides_of_page_boundary() -> AnyResult<()> {
        use alloy::transports::mock::Asserter;
        let asserter = Asserter::new();
        let contracts = mocked_ancestry_api(asserter.clone())?.contracts;
        for (height, source, hash) in [(100_006u64, 12u64, 1u8), (100_007, 13, 2)] {
            let event = MerkleRoot {
                blockNumber: U256::from(source),
                merkleRoot: B256::from([hash; 32]),
            };
            let data = event.encode_log_data();
            let log: RpcLog = serde_json::from_value(serde_json::json!({
                "address": Address::from([9; 20]),
                "topics": data.topics(), "data": data.data,
                "blockNumber": format!("0x{height:x}"), "blockHash": B256::from([hash; 32]),
                "transactionHash": B256::from([hash + 10; 32]), "transactionIndex": "0x0",
                "logIndex": "0x0", "removed": false
            }))?;
            asserter.push_success(&vec![log]);
        }
        let roots = contracts.fetch_merkle_roots_in_range(7, 100_007).await?;
        assert_eq!(
            roots
                .iter()
                .map(|root| (root.block_number, root.entry.block_number))
                .collect::<Vec<_>>(),
            vec![(100_006, 12), (100_007, 13)]
        );
        assert!(asserter.read_q().is_empty());
        // An inclusive page ending at the integer limit must terminate without overflow.
        asserter.push_success(&Vec::<RpcLog>::new());
        assert!(contracts
            .fetch_merkle_roots_in_range(u64::MAX - 99_999, u64::MAX)
            .await?
            .is_empty());
        assert!(asserter.read_q().is_empty());
        assert!(contracts
            .fetch_merkle_roots_in_range(9, 8)
            .await?
            .is_empty());
        Ok(())
    }

    #[test]
    fn merkle_root_metadata_is_required_and_source_height_is_checked() {
        let event = MerkleRoot {
            blockNumber: U256::from(12),
            merkleRoot: B256::from([3; 32]),
        };
        let value = serde_json::json!({
            "address": Address::ZERO, "topics": [], "data": "0x",
            "blockNumber": "0x7", "blockHash": B256::from([1; 32]),
            "transactionHash": B256::from([2; 32]), "transactionIndex": "0x0",
            "logIndex": "0x1", "removed": false
        });
        let log: RpcLog = serde_json::from_value(value.clone()).unwrap();
        let observation = merkle_root_observation(&event, &log, 7, 7).unwrap();
        assert_eq!(observation.entry.block_number, 12);
        assert_eq!(observation.transaction_index, 0);
        for field in [
            "blockNumber",
            "blockHash",
            "transactionHash",
            "transactionIndex",
            "logIndex",
        ] {
            let mut missing = value.clone();
            missing[field] = Value::Null;
            let log: RpcLog = serde_json::from_value(missing).unwrap();
            assert!(
                matches!(
                    merkle_root_observation(&event, &log, 7, 7),
                    Err(Error::InvalidMerkleRootObservation(_))
                ),
                "{field}"
            );
        }
        assert!(merkle_root_observation(&event, &log, 8, 9).is_err());
        let overflow = MerkleRoot {
            blockNumber: U256::from(u32::MAX) + U256::from(1),
            merkleRoot: event.merkleRoot,
        };
        assert!(merkle_root_observation(&overflow, &log, 7, 7).is_err());
        let mut removed = log;
        removed.removed = true;
        assert!(
            merkle_root_observation(&event, &removed, 7, 7)
                .unwrap()
                .removed
        );
        validate_merkle_root_observations(&[observation, observation]).unwrap();
        for field in ["block", "transaction", "transaction_index", "root"] {
            let mut conflicting = observation;
            match field {
                "block" => conflicting.block_hash = B256::from([9; 32]),
                "transaction" => conflicting.transaction_hash = B256::from([9; 32]),
                "transaction_index" => conflicting.transaction_index += 1,
                "root" => conflicting.entry.merkle_root = H256::repeat_byte(9),
                _ => unreachable!(),
            }
            assert!(
                validate_merkle_root_observations(&[observation, conflicting]).is_err(),
                "{field}"
            );
        }
        let encoded = serde_json::to_value(observation).unwrap();
        for field in [
            "entry",
            "block_number",
            "block_hash",
            "transaction_hash",
            "transaction_index",
            "log_index",
            "removed",
        ] {
            let mut missing = encoded.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(
                serde_json::from_value::<MerkleRootObservation>(missing).is_err(),
                "{field}"
            );
        }
        let mut extra = encoded;
        extra["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<MerkleRootObservation>(extra).is_err());
    }

    #[test]
    fn finalized_submission_snapshot_serde_requires_exact_evidence_fields() {
        let snapshot = FinalizedSubmissionSnapshot {
            block_number: 100,
            block_hash: B256::from([7; 32]),
            pinned_nonce_consumed: true,
            message_processed: false,
        };
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(
            serde_json::from_value::<FinalizedSubmissionSnapshot>(value.clone()).unwrap(),
            snapshot
        );
        for field in [
            "block_number",
            "block_hash",
            "pinned_nonce_consumed",
            "message_processed",
        ] {
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<FinalizedSubmissionSnapshot>(missing).is_err());
        }
        let mut extra = value;
        extra["unexpected"] = serde_json::json!(true);
        assert!(serde_json::from_value::<FinalizedSubmissionSnapshot>(extra).is_err());
    }

    pub(super) static ANCESTRY_TEST_LOCK: Mutex<()> = Mutex::const_new(());

    pub(super) struct ClearOwnedChains<const N: usize>(pub(super) [B256; N]);
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

    pub(super) struct Cleanup {
        pub(super) directory: std::path::PathBuf,
        pub(super) genesis: B256,
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
    pub(super) fn header_chain(seed: u8, height: u64) -> Vec<Block> {
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
            queue_deployment_transaction: None,
            chain_id: 560048,
            public_key,
            wallet,
            url: Url::parse("ws://127.0.0.1:1")?,
            ws_max_retry: None,
            ws_retry_interval: None,
            submission_lock: submission_lock(560048, public_key),
        })
    }

    #[tokio::test]
    async fn challenge_submission_distinguishes_estimation_from_send_failure() -> AnyResult<()> {
        use alloy::transports::mock::Asserter;

        let asserter = Asserter::new();
        let api = mocked_ancestry_api(asserter.clone())?;
        asserter.push_failure_msg("estimate rejected");
        asserter.push_success(&"0x5208");
        assert!(matches!(
            api.send_challenge_root().await,
            Err(Error::ChallengeRootPreBroadcast(_))
        ));
        assert_eq!(asserter.read_q().len(), 1, "estimation must not send");

        // The remaining successful estimate gets as far as send. Its missing
        // RPC replies must not be mistaken for a retryable estimation error.
        assert!(matches!(
            api.send_challenge_root().await,
            Err(Error::ErrorSendingTransaction(_))
        ));
        assert!(asserter.read_q().is_empty());
        Ok(())
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

    // Exercise the ancestry/cache algorithm independently of finality qualification.
    // Production acceptance and receipt proof regressions use two transports in finality_tests.
    async fn ancestry_only(
        provider: &impl Provider,
        number: u64,
        hash: B256,
    ) -> Result<Option<(u64, B256)>, Error> {
        let started = Instant::now();
        let head = qualified_header(provider, BlockNumberOrTag::Finalized).await?;
        let genesis = qualified_header(provider, BlockNumberOrTag::Number(0)).await?;
        finalized_ancestor_at(provider, number, hash, &head, genesis.hash, started, None).await
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
        assert_eq!(ancestry_only(&provider, 100, b_inclusion).await?, None);
        asserter.push_success(&head);
        asserter.push_success(&genesis);
        assert_eq!(
            ancestry_only(&provider, 100, a_inclusion).await?,
            Some((102, a_head))
        );

        let fork = block(103, 26, B256::from([27; 32]));
        asserter.push_success(&fork);
        asserter.push_success(&genesis);
        asserter.push_success(&fork);
        assert_eq!(
            ancestry_only(&provider, 100, a_inclusion).await?,
            None,
            "a newer finalized tag on another branch must not reuse cached ancestry"
        );
        let extension = block(103, 28, a_head);
        asserter.push_success(&extension);
        asserter.push_success(&genesis);
        asserter.push_success(&extension);
        assert_eq!(
            ancestry_only(&provider, 100, a_inclusion).await?,
            Some((103, extension.header.hash))
        );

        let other_head = block(102, 31, B256::ZERO);
        let other_genesis = block(0, 32, B256::ZERO);
        asserter.push_success(&other_head);
        asserter.push_success(&other_genesis);
        assert_eq!(
            ancestry_only(&provider, 102, other_head.header.hash).await?,
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
            ancestry_only(&provider, 0, hash(0)).await?,
            Some((258, hash(258)))
        );
        // Subsequent historical receipts share the already verified lineage.
        asserter.push_success(&head);
        asserter.push_success(&genesis);
        assert_eq!(
            ancestry_only(&provider, 1, hash(1)).await?,
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
            ancestry_only(&provider, 0, hash(0)).await,
            Err(Error::FinalizedAncestryPending)
        ));
        asserter.push_success(&block(21));
        asserter.push_success(&block(0));
        asserter.push_success(&block(21));
        asserter.push_success(&block(2));
        asserter.push_success(&block(1));
        assert_eq!(
            ancestry_only(&provider, 0, hash(0)).await?,
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
            ancestry_only(&provider, 20, hash(20)).await?,
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
            ancestry_only(&provider, 40, hash(40)).await,
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
            ancestry_only(&provider, 40, hash(40)).await?,
            Some((41, hash(41)))
        );
        assert!(asserter.read_q().is_empty());
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
        assert!(ancestry_only(&provider, 0, chain[0].header.hash)
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
            ancestry_only(&provider, target, chain[target as usize].header.hash).await,
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
            ancestry_only(&provider, newer, chain[newer as usize].header.hash)
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
            ancestry_only(&provider, target, chain[target as usize].header.hash)
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
    #[tokio::test]
    #[ignore = "requires Anvil; signs and mines a local fee-envelope transaction"]
    async fn content_message_follows_market_fees_without_fixed_tip() -> AnyResult<()> {
        use alloy::{
            consensus::{Transaction, TxEnvelope},
            eips::Decodable2718,
            node_bindings::Anvil,
        };
        let anvil = Anvil::new()
            .host("127.0.0.1")
            .port(0u16)
            .args(["--base-fee", "1", "--gas-price", "1"])
            .try_spawn()?;
        let private_key = format!("{:#x}", B256::from_slice(&anvil.keys()[0].to_bytes()));
        // Fee/signature smoke only: an EOA avoids conflating queue settlement with fee policy.
        let api = EthApi::new(
            &anvil.endpoint(),
            &format!("{:#x}", anvil.addresses()[1]),
            Some(&private_key),
            None,
            None,
        )
        .await?;
        let quote = api.raw_provider().estimate_eip1559_fees().await?;
        ensure!(
            quote.max_priority_fee_per_gas < 500_000_000,
            "fixture must quote below the old tip floor"
        );
        let guard = api.reserve_submission().await;
        let prepared = api
            .prepare_content_message(&guard, 1, 1, 0, [0; 32], [1; 32], [2; 20], vec![], vec![])
            .await?;
        let signed = TxEnvelope::decode_2718(&mut prepared.raw_transaction.as_slice())?;
        assert_eq!(
            signed.max_priority_fee_per_gas(),
            Some(quote.max_priority_fee_per_gas)
        );
        assert_eq!(signed.max_fee_per_gas(), quote.max_fee_per_gas);
        let hash = api
            .broadcast_prepared_content_message(&guard, &prepared)
            .await?;
        let receipt = api
            .raw_provider()
            .get_transaction_receipt(hash)
            .await?
            .context("local original receipt missing")?;
        assert_eq!(receipt.transaction_hash, prepared.hash);
        assert!(receipt.status());
        assert!(receipt.effective_gas_price < 500_000_000);
        println!(
            "signed original {hash:#x}: gas={}, effectiveGasPrice={} wei",
            receipt.gas_used, receipt.effective_gas_price
        );
        Ok(())
    }
}
