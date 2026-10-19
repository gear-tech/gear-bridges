#[cfg(test)]
use alloy::node_bindings::Anvil;
use alloy::{
    consensus::{transaction::SignerRecoverable, Transaction as _, TxEnvelope},
    eips::{Decodable2718, Encodable2718},
    node_bindings::AnvilInstance,
    primitives::{Address, Bytes, FixedBytes, B256, U256},
    providers::{Provider, WalletProvider},
    rpc::types::{BlockId, BlockNumberOrTag},
    signers::local::PrivateKeySigner,
    sol,
};
use anyhow::{ensure, Context, Result};
use ethereum_client::{transaction_identity, EthApi, PollingEthApi, TransactionIdentity};
use parity_scale_codec::{Decode, Encode};
use serde_json::{json, Value};
use std::{collections::BTreeSet, fs, path::Path, str::FromStr, time::Duration};
#[cfg(test)]
use std::{path::PathBuf, process::Stdio};
#[cfg(test)]
use tokio::process::Command;
use tokio::time::{sleep, timeout, Instant};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::{
    authority_proof, keccak256, source::CapturedCommitment, Hash32, QueueSnapshot, RuntimeLeaf,
    SimplifiedMmrProof, SNAPSHOT_VERSION,
};

fn bytes(bytes: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(bytes))
}

pub(crate) fn signed_transaction_identity(
    raw: &[u8],
    hash: B256,
    nonce: u64,
    destination: Address,
) -> Result<TransactionIdentity> {
    ensure!(
        B256::from(keccak256(raw)) == hash,
        "saved signed transaction bytes do not match their hash"
    );
    let mut encoded = raw;
    let signed =
        TxEnvelope::decode_2718(&mut encoded).context("invalid saved signed transaction")?;
    ensure!(
        encoded.is_empty(),
        "saved signed transaction has trailing bytes"
    );
    ensure!(
        signed.nonce() == nonce && signed.to() == Some(destination),
        "saved signed transaction nonce or destination changed"
    );
    Ok(TransactionIdentity {
        hash,
        from: signed
            .recover_signer()
            .context("invalid saved transaction signature")?,
        nonce,
        to: Some(destination),
        block_number: None,
    })
}
pub(crate) fn ensure_same_submission(
    observed: TransactionIdentity,
    signed: TransactionIdentity,
) -> Result<()> {
    ensure!(
        observed.hash == signed.hash
            && observed.from == signed.from
            && observed.nonce == signed.nonce
            && observed.to == signed.to,
        "RPC transaction identity differs from original signed bytes"
    );
    Ok(())
}
async fn await_original_commitment(
    provider: &impl Provider,
    signed: TransactionIdentity,
    raw: &[u8],
) -> Result<alloy::rpc::types::TransactionReceipt> {
    timeout(Duration::from_secs(120), async {
        let mut last_broadcast: Option<Instant> = None;
        loop {
            if let Some(receipt) = provider.get_transaction_receipt(signed.hash).await? {
                ensure!(
                    receipt.transaction_hash == signed.hash
                        && receipt.from == signed.from
                        && receipt.to == signed.to,
                    "commitment receipt identity differs from signed intent"
                );
                let block = receipt.block_number.context("commitment receipt block missing")?;
                let hash = receipt.block_hash.context("commitment receipt inclusion hash missing")?;
                wait_canonical_receipt(provider, block, hash).await?;
                return Ok(receipt);
            }
            if let Some(observed) = transaction_identity(provider, signed.hash).await? {
                ensure_same_submission(observed, signed)?;
            } else {
                let latest = provider.get_transaction_count(signed.from).await?;
                let pending = provider.get_transaction_count(signed.from).pending().await?;
                ensure!(
                    latest == signed.nonce && pending == signed.nonce,
                    "HOLD: original commitment {} disappeared while nonce {} is uncertain (latest={latest}, pending={pending})",
                    signed.hash, signed.nonce,
                );
                if last_broadcast.is_none_or(|at| at.elapsed() >= Duration::from_secs(30)) {
                    let sent = provider.send_raw_transaction(raw).await.context(
                        "HOLD: rebroadcast of saved original commitment failed; retain raw bytes/hash/nonce",
                    )?;
                    ensure!(*sent.tx_hash() == signed.hash, "HOLD: RPC returned a different commitment hash");
                    last_broadcast = Some(Instant::now());
                }
            }
            sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .with_context(|| format!(
        "HOLD: original commitment {} nonce {} has no canonical receipt after 120 seconds; retain original signed bytes and reconcile before restart",
        signed.hash, signed.nonce,
    ))?
}

pub(crate) struct OriginalCommitmentReceipt {
    pub hash: B256,
    pub raw: Vec<u8>,
    pub source_block: u32,
    pub mmr_root: Hash32,
    pub client: Address,
}

pub(crate) fn validate_recorded_commitment_transaction(
    raw: &[u8],
    source_raw: &[u8],
) -> Result<()> {
    use alloy::sol_types::{SolCall, SolValue};
    let mut encoded = raw;
    let signed = TxEnvelope::decode_2718(&mut encoded)?;
    ensure!(
        encoded.is_empty(),
        "original commitment transaction has trailing bytes; HOLD"
    );
    let call = BeefyClient::submitFiatShamirCall::abi_decode(signed.input())
        .context("original transaction is not the saved Fiat-Shamir submission; HOLD")?;
    let original = crate::decode_versioned_finality_proof(source_raw)?;
    let expected = BeefyClient::Commitment {
        blockNumber: original.commitment.block_number,
        validatorSetID: original.commitment.validator_set_id,
        payload: commitment_payload(&original.commitment.payload)?,
    };
    ensure!(
        call.commitment.abi_encode() == expected.abi_encode(),
        "original signed transaction changed source commitment semantics; HOLD"
    );
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RecordedReceiptProof {
    genesis: B256,
    pub(crate) header: alloy::consensus::Header,
    pub(crate) index: u64,
    pub(crate) raw_transaction: Vec<u8>,
    pub(crate) receipt: alloy::consensus::ReceiptEnvelope,
    transaction_proof: Vec<Vec<u8>>,
    receipt_proof: Vec<Vec<u8>>,
}

fn verify_indexed_value(root: B256, index: u64, value: &[u8], proof: &[Vec<u8>]) -> Result<()> {
    use ethereum_common::{
        hash_db, memory_db,
        patricia_trie::TrieDB,
        trie_db::{HashDB, Trie},
        utils,
    };
    let mut database = memory_db::new();
    for node in proof {
        database.insert(hash_db::EMPTY_PREFIX, node);
    }
    let root = ethereum_common::H256(root.0);
    let trie = TrieDB::new(&database, &root)
        .map_err(|_| anyhow::anyhow!("invalid original inclusion proof database; HOLD"))?;
    let key = utils::rlp_encode_transaction_index(&index);
    ensure!(
        trie.get(&key)
            .map_err(|_| anyhow::anyhow!("unavailable original inclusion proof node; HOLD"))?
            .as_deref()
            == Some(value),
        "original transaction/receipt inclusion proof is invalid; HOLD"
    );
    Ok(())
}

fn transaction_proof(
    transactions: &[TxEnvelope],
    index: u64,
    expected_root: B256,
) -> Result<Vec<Vec<u8>>> {
    use ethereum_common::{
        memory_db,
        patricia_trie::{TrieDB, TrieDBMut},
        trie_db::{Recorder, Trie, TrieMut},
        utils,
    };
    let mut database = memory_db::new();
    let mut root = ethereum_common::H256::zero();
    {
        let mut trie = TrieDBMut::new(&mut database, &mut root);
        for (index, transaction) in transactions.iter().enumerate() {
            trie.insert(
                &utils::rlp_encode_transaction_index(&(index as u64)),
                &transaction.encoded_2718(),
            )
            .map_err(|_| {
                anyhow::anyhow!("original transaction trie reconstruction failed; HOLD")
            })?;
        }
        ensure!(
            trie.root().as_bytes() == expected_root.as_slice(),
            "full block transactions do not match the authenticated header root; HOLD"
        );
    }
    let trie = TrieDB::new(&database, &root)
        .map_err(|_| anyhow::anyhow!("invalid original transaction trie; HOLD"))?;
    let mut recorder = Recorder::new();
    let expected = transactions
        .get(usize::try_from(index)?)
        .context("original transaction index is absent")?
        .encoded_2718();
    ensure!(
        trie.get_with(&utils::rlp_encode_transaction_index(&index), &mut recorder)
            .map_err(|_| anyhow::anyhow!("original transaction trie proof failed; HOLD"))?
            .as_deref()
            == Some(expected.as_slice()),
        "original transaction trie returned different bytes; HOLD"
    );
    Ok(recorder
        .drain()
        .into_iter()
        .map(|record| record.data)
        .collect())
}

impl RecordedReceiptProof {
    pub(crate) fn verify_inclusion(
        &self,
        view: &ethereum_client::FinalizedView,
        hash: B256,
    ) -> Result<()> {
        ensure!(
            self.genesis == view.genesis_hash()
                && view.contains(self.header.number, self.header.hash_slow()),
            "receipt proof is not bound to authenticated finalized history; HOLD"
        );
        ensure!(
            B256::from(keccak256(&self.raw_transaction)) == hash,
            "receipt proof transaction hash changed; HOLD"
        );
        verify_indexed_value(
            self.header.transactions_root,
            self.index,
            &self.raw_transaction,
            &self.transaction_proof,
        )?;
        verify_indexed_value(
            self.header.receipts_root,
            self.index,
            &self.receipt.encoded_2718(),
            &self.receipt_proof,
        )
    }
    fn verify(
        &self,
        view: &ethereum_client::FinalizedView,
        original: &OriginalCommitmentReceipt,
    ) -> Result<()> {
        let raw = original.raw.as_slice();
        let hash = original.hash;
        let header_hash = self.header.hash_slow();
        ensure!(
            self.genesis == view.genesis_hash() && view.contains(self.header.number, header_hash),
            "original receipt proof is not bound to this authenticated finalized history; HOLD"
        );
        ensure!(
            self.raw_transaction == raw && B256::from(keccak256(raw)) == hash,
            "original receipt proof changed signed transaction identity; HOLD"
        );
        verify_indexed_value(
            self.header.transactions_root,
            self.index,
            raw,
            &self.transaction_proof,
        )?;
        verify_indexed_value(
            self.header.receipts_root,
            self.index,
            &self.receipt.encoded_2718(),
            &self.receipt_proof,
        )?;
        ensure!(
            self.receipt.status(),
            "original commitment transaction reverted; HOLD"
        );
        let event = B256::from(keccak256(b"NewMMRRoot(bytes32,uint64)"));
        ensure!(
            self.receipt
                .logs()
                .iter()
                .any(|log| log.address == original.client
                    && log.data.topics() == [event]
                    && log.data.data.len() == 64
                    && log.data.data[..32] == original.mmr_root
                    && U256::from_be_slice(&log.data.data[32..])
                        == U256::from(original.source_block)),
            "original authenticated receipt did not accept its source MMR commitment; HOLD"
        );
        Ok(())
    }
}

/// Authenticate arbitrary signed transaction and receipt bytes; policy belongs to the caller.
pub(crate) async fn authenticate_transaction_receipt(
    ethereum: &Ethereum,
    view: &ethereum_client::FinalizedView,
    block_number: u64,
    block_hash: B256,
    hash: B256,
    directory: &Path,
) -> Result<RecordedReceiptProof> {
    fs::create_dir_all(directory)?;
    let path = directory.join(format!("{hash:x}.json"));
    if let Ok(saved) = fs::read(&path) {
        let proof: RecordedReceiptProof = serde_json::from_slice(&saved)?;
        ensure!(
            proof.header.number == block_number && proof.header.hash_slow() == block_hash,
            "saved receipt inclusion changed; HOLD"
        );
        proof.verify_inclusion(view, hash)?;
        return Ok(proof);
    }
    ensure!(
        view.contains(block_number, block_hash),
        "receipt block is not authenticated; HOLD"
    );
    let (block, receipts) = tokio::try_join!(
        async {
            ethereum
                .api
                .raw_provider()
                .get_block_by_hash(block_hash)
                .full()
                .await?
                .context("full inclusion block unavailable; HOLD")
        },
        async {
            ethereum
                .api
                .raw_provider()
                .get_block_receipts(BlockId::hash_canonical(block_hash))
                .await?
                .context("inclusion receipts unavailable; HOLD")
        },
    )?;
    ensure!(
        block.header.number == block_number
            && block.header.hash == block_hash
            && block.header.inner.hash_slow() == block_hash,
        "inclusion header changed; HOLD"
    );
    let transactions = block
        .transactions
        .try_into_transactions()
        .map_err(|_| anyhow::anyhow!("full transactions missing; HOLD"))?
        .into_iter()
        .map(|tx| tx.into_inner())
        .collect::<Vec<_>>();
    ensure!(
        receipts.len() == transactions.len(),
        "incomplete receipt set; HOLD"
    );
    let mut envelopes = Vec::with_capacity(receipts.len());
    for (index, receipt) in receipts.into_iter().enumerate() {
        ensure!(
            receipt.transaction_index == Some(index as u64)
                && receipt.block_number == Some(block_number)
                && receipt.block_hash == Some(block_hash)
                && receipt.transaction_hash
                    == B256::from(keccak256(&transactions[index].encoded_2718())),
            "receipt coordinates changed; HOLD"
        );
        envelopes.push((index as u64, receipt.into_primitives_receipt().inner));
    }
    ensure!(
        alloy::consensus::proofs::calculate_receipt_root(
            &envelopes
                .iter()
                .map(|(_, receipt)| receipt)
                .collect::<Vec<_>>()
        ) == block.header.receipts_root,
        "receipt trie differs from authenticated header; HOLD"
    );
    let index = transactions
        .iter()
        .position(|tx| B256::from(keccak256(&tx.encoded_2718())) == hash)
        .context("signed transaction absent from inclusion; HOLD")? as u64;
    let receipt = ethereum_common::utils::generate_merkle_proof(index, &envelopes)?;
    let proof = RecordedReceiptProof {
        genesis: view.genesis_hash(),
        header: block.header.inner.clone(),
        index,
        raw_transaction: transactions[index as usize].encoded_2718(),
        receipt: receipt.receipt,
        receipt_proof: receipt.proof,
        transaction_proof: transaction_proof(&transactions, index, block.header.transactions_root)?,
    };
    proof.verify_inclusion(view, hash)?;
    let temporary = path.with_extension("json.tmp");
    use std::io::Write;
    let mut file = std::io::BufWriter::new(fs::File::create(&temporary)?);
    serde_json::to_writer(&mut file, &proof)?;
    file.flush()?;
    file.get_ref().sync_all()?;
    fs::rename(temporary, path)?;
    fs::File::open(directory)?.sync_all()?;
    Ok(proof)
}

/// Fetch a block once for every distinct original inclusion, then retain compact
/// transaction and receipt MPT proofs. Receipt JSON metadata is never reused as proof.
pub(crate) async fn authenticate_recorded_receipts(
    ethereum: &Ethereum,
    view: &ethereum_client::FinalizedView,
    block_number: u64,
    block_hash: B256,
    originals: &[OriginalCommitmentReceipt],
    directory: &Path,
) -> Result<()> {
    fs::create_dir_all(directory)?;
    let mut missing = Vec::new();
    for original in originals {
        let path = directory.join(format!("{:x}.json", original.hash));
        let stored = fs::metadata(&path)
            .ok()
            .filter(|metadata| metadata.len() <= 64 * 1024 * 1024)
            .and_then(|_| fs::read(&path).ok())
            .and_then(|bytes| serde_json::from_slice::<RecordedReceiptProof>(&bytes).ok());
        if stored.as_ref().is_some_and(|proof| {
            proof.header.number == block_number
                && proof.header.hash_slow() == block_hash
                && proof.verify(view, original).is_ok()
        }) {
            continue;
        }
        missing.push((original, path));
    }
    if missing.is_empty() {
        return Ok(());
    }
    ensure!(
        view.contains(block_number, block_hash),
        "original receipt inclusion has not been authenticated; HOLD"
    );
    let (block, receipts) = tokio::try_join!(
        async {
            ethereum
                .api
                .raw_provider()
                .get_block_by_hash(block_hash)
                .full()
                .await?
                .context("original full block is unavailable; HOLD")
        },
        async {
            ethereum
                .api
                .raw_provider()
                .get_block_receipts(BlockId::hash_canonical(block_hash))
                .await?
                .context("original block receipts are unavailable; HOLD")
        },
    )?;
    ensure!(
        block.header.number == block_number
            && block.header.hash == block_hash
            && block.header.inner.hash_slow() == block_hash,
        "original full block changed its authenticated header; HOLD"
    );
    let transactions = block
        .transactions
        .try_into_transactions()
        .map_err(|_| anyhow::anyhow!("original block omitted full transactions; HOLD"))?
        .into_iter()
        .map(|transaction| transaction.into_inner())
        .collect::<Vec<_>>();
    ensure!(
        receipts.len() == transactions.len(),
        "original block omitted transaction receipts; HOLD"
    );
    let mut envelopes = Vec::with_capacity(receipts.len());
    for (index, receipt) in receipts.into_iter().enumerate() {
        ensure!(
            receipt.transaction_index == Some(index as u64)
                && receipt.block_number == Some(block_number)
                && receipt.block_hash == Some(block_hash)
                && receipt.transaction_hash
                    == B256::from(keccak256(&transactions[index].encoded_2718())),
            "original receipt block/index/transaction identity changed; HOLD"
        );
        envelopes.push((index as u64, receipt.into_primitives_receipt().inner));
    }
    ensure!(
        alloy::consensus::proofs::calculate_receipt_root(
            &envelopes
                .iter()
                .map(|(_, receipt)| receipt)
                .collect::<Vec<_>>()
        ) == block.header.receipts_root,
        "original block receipt trie does not match its authenticated header; HOLD"
    );
    for (original, path) in missing {
        let index = transactions
            .iter()
            .position(|transaction| transaction.encoded_2718() == original.raw)
            .context(
                "original signed transaction is absent from its authenticated inclusion; HOLD",
            )? as u64;
        let receipt = ethereum_common::utils::generate_merkle_proof(index, &envelopes)?;
        let proof = RecordedReceiptProof {
            genesis: view.genesis_hash(),
            header: block.header.inner.clone(),
            index,
            raw_transaction: original.raw.clone(),
            receipt: receipt.receipt,
            receipt_proof: receipt.proof,
            transaction_proof: transaction_proof(
                &transactions,
                index,
                block.header.transactions_root,
            )?,
        };
        proof.verify(view, original)?;
        if path.exists() {
            let held = path.with_extension(format!(
                "held-{}.json",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos()
            ));
            fs::rename(&path, held)?;
            fs::File::open(directory)?.sync_all()?;
        }
        let temporary = path.with_extension("json.tmp");
        let mut file = std::io::BufWriter::new(fs::File::create(&temporary)?);
        serde_json::to_writer(&mut file, &proof)?;
        use std::io::Write;
        file.flush()?;
        file.get_ref().sync_all()?;
        fs::rename(temporary, &path)?;
        fs::File::open(directory)?.sync_all()?;
    }
    Ok(())
}

const HOODI_CHAIN_ID: u64 = 560_048;
const HOODI_GENESIS_BLOCK: B256 =
    alloy::primitives::b256!("bbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b");
const MAINNET_GENESIS_BLOCK: B256 =
    alloy::primitives::b256!("d4e56740f876aef8c010b86a40d5f56745a118d0906a34e69aec8c0db1cb8fa3");
const HOODI_DEFAULT_RPC: &str = "https://hoodi.drpc.org";

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum PublicEthereumNetwork {
    Hoodi,
    Mainnet,
}

impl PublicEthereumNetwork {
    fn chain_id(self) -> u64 {
        match self {
            Self::Hoodi => HOODI_CHAIN_ID,
            Self::Mainnet => 1,
        }
    }

    fn genesis(self) -> B256 {
        match self {
            Self::Hoodi => HOODI_GENESIS_BLOCK,
            Self::Mainnet => MAINNET_GENESIS_BLOCK,
        }
    }

    pub(crate) fn deployment_mode(self) -> &'static str {
        match self {
            Self::Hoodi => "hoodi-token-stack",
            Self::Mainnet => "mainnet-token-stack",
        }
    }

    pub(crate) fn follower_mode(self) -> &'static str {
        match self {
            Self::Hoodi => "hoodi-token-follow",
            Self::Mainnet => "mainnet-token-follow",
        }
    }
}

/// Independent offline EL/Beacon genesis approval, never inferred from an RPC.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OwnedFixtureNetwork {
    pub schema_version: u32,
    pub ethereum_chain_id: u64,
    pub ethereum_genesis_hash: B256,
    pub beacon_genesis_validators_root: B256,
    pub beacon_genesis_time: u64,
}

impl OwnedFixtureNetwork {
    pub fn read(path: &Path) -> Result<Self> {
        let network: Self = serde_json::from_slice(&fs::read(path)?)?;
        network.validate()?;
        Ok(network)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.ethereum_chain_id == 31_337,
            "owned fixture requires schema 1 and Ethereum chain 31337; HOLD"
        );
        ensure!(
            self.ethereum_genesis_hash != B256::ZERO
                && self.beacon_genesis_validators_root != B256::ZERO
                && self.beacon_genesis_time > 0,
            "owned fixture genesis approval is incomplete; HOLD"
        );
        Ok(())
    }

    pub fn validate_endpoints(&self, source: &str, witness: &str, ethereum: &str) -> Result<()> {
        self.validate()?;
        ensure!(
            source != witness,
            "owned fixture requires independent source/witness endpoints"
        );
        for endpoint in [source, witness, ethereum] {
            self.validate_endpoint(endpoint)?;
        }
        Ok(())
    }

    fn validate_endpoint(&self, endpoint: &str) -> Result<()> {
        self.validate()?;
        ensure!(
            crate::local_source_rpc(endpoint)
                && matches!(
                    url::Url::parse(endpoint)?.host(),
                    Some(url::Host::Ipv4(_) | url::Host::Ipv6(_))
                ),
            "owned fixture endpoints must be numeric loopback websocket authorities; HOLD"
        );
        Ok(())
    }

    pub fn validate_manifest(&self, deployment: &Value) -> Result<()> {
        self.validate()?;
        let persisted: Self = serde_json::from_value(deployment["ownedFixtureNetwork"].clone())
            .context("owned fixture approval missing from deployment; HOLD")?;
        ensure!(
            persisted == *self
                && deployment["localRehearsal"] == false
                && manifest_u64(&deployment["ethereum"], "chainId")? == self.ethereum_chain_id,
            "owned fixture approval differs from immutable deployment; HOLD"
        );
        Ok(())
    }
}

pub fn approved_fixture_network(
    path: Option<&Path>,
    deployment: &Value,
    source: &str,
    witness: &str,
    ethereum: &str,
    public_network: PublicEthereumNetwork,
) -> Result<Option<OwnedFixtureNetwork>> {
    let network = path.map(OwnedFixtureNetwork::read).transpose()?;
    validate_fixture_approval(
        network.as_ref(),
        deployment,
        source,
        witness,
        ethereum,
        public_network,
    )?;
    Ok(network)
}

pub(crate) fn validate_fixture_approval(
    network: Option<&OwnedFixtureNetwork>,
    deployment: &Value,
    source: &str,
    witness: &str,
    ethereum: &str,
    public_network: PublicEthereumNetwork,
) -> Result<()> {
    if public_network == PublicEthereumNetwork::Mainnet {
        ensure!(
            network.is_none() && deployment["localRehearsal"] == false,
            "mainnet cannot use an owned fixture or local rehearsal; HOLD"
        );
        ensure!(
            deployment["mode"] == public_network.deployment_mode(),
            "mainnet requires an independently reviewed mainnet-token-stack manifest; HOLD"
        );
        ensure!(
            ethereum.starts_with("https://") || ethereum.starts_with("wss://"),
            "mainnet requires HTTPS or WSS; HOLD"
        );
    }
    if let Some(network) = network {
        network.validate_endpoints(source, witness, ethereum)?;
        network.validate_manifest(deployment)?;
    } else {
        ensure!(deployment.get("ownedFixtureNetwork").is_none(),
            "owned fixture requires explicit --owned-fixture-network approval on every attachment; HOLD");
        ensure!(
            manifest_u64(&deployment["ethereum"], "chainId")? == public_network.chain_id(),
            "deployment differs from explicitly selected public network; HOLD"
        );
    }
    Ok(())
}

sol!(
    #![sol(rpc, extra_derives(Debug))]
    #[allow(clippy::too_many_arguments)]
    BeefyClient,
    "../../api/ethereum/BeefyClient.json"
);

sol!(
    #![sol(rpc, extra_derives(Debug))]
    VaraQueueRootVerifier,
    "../../api/ethereum/VaraQueueRootVerifier.json"
);

sol! {
    #[sol(rpc)]
    interface TokenManagerBinding {
        function messageQueue() external view returns (address);
        function vftManagers() external view returns (bytes32[] memory);
    }
}
sol! {
    #[sol(rpc)]
    interface TokenQueueBinding {
        function verifier() external view returns (address);
        function maxBlockNumber() external view returns (uint256);
        function beefyRootMinimum() external view returns (uint256);
        function beefyRecoveryTimelock() external view returns (address);
    }
    #[sol(rpc)]
    interface RecoveryTimelockBinding {
        function getMinDelay() external view returns (uint256);
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationCheckpoint {
    pub block: u64,
    pub root: Hash32,
    pub source_timestamp_ms: u64,
    pub current_id: u64,
    pub current_len: u64,
    pub current_root: Hash32,
    pub next_id: u64,
    pub next_len: u64,
    pub next_root: Hash32,
}

pub struct Ethereum {
    _anvil: Option<AnvilInstance>,
    pub(crate) owned_fixture_network: Option<OwnedFixtureNetwork>,
    pub(crate) public_network: PublicEthereumNetwork,
    pub api: EthApi,
    pub client_address: Address,
    pub verifier_address: Address,
    pub queue_address: Address,
    pub receiver_address: Address,
    pub transactions: Vec<Value>,
}

impl Ethereum {
    #[cfg(test)]
    pub async fn prepare(
        output: &Path,
        checkpoint: &CapturedCommitment,
        snapshot: &QueueSnapshot,
        mmr_start_block: u64,
    ) -> Result<Self> {
        let initial_block = u64::from(checkpoint.block);
        ensure!(
            mmr_start_block > 0 && initial_block > mmr_start_block,
            "bootstrap BEEFY checkpoint must be after MMR history start"
        );
        ensure!(
            checkpoint.finalized_height >= checkpoint.block,
            "bootstrap checkpoint is not finalized"
        );
        ensure!(
            checkpoint.validated.mmr_root != [0; 32],
            "bootstrap BEEFY checkpoint has a zero MMR root"
        );
        ensure!(
            snapshot.bridge_domain != [0; 32] && snapshot.source_timestamp_ms != 0,
            "bootstrap metadata has no source identity or timestamp"
        );
        ensure!(
            !checkpoint.current.keys.is_empty()
                && checkpoint.current.keys.len() <= 256
                && !checkpoint.next.keys.is_empty()
                && checkpoint.next.keys.len() <= 256
                && checkpoint.current.id.checked_add(1) == Some(checkpoint.next.id),
            "bootstrap authority sets are invalid"
        );
        let timestamp = snapshot.source_timestamp_ms / 1_000;
        ensure!(
            timestamp > 0,
            "bootstrap source timestamp is before Unix epoch"
        );

        let anvil = Anvil::new()
            .mnemonic("test test test test test test test test test test test junk")
            .host("127.0.0.1")
            .port(0u16)
            .chain_id(31_337)
            .block_time(1)
            .arg("--slots-in-an-epoch")
            .arg("1")
            .arg("--timestamp")
            .arg(timestamp.to_string())
            .try_spawn()
            .context("spawn loopback Anvil")?;
        let private_key = format!("0x{}", hex::encode(anvil.first_key().to_bytes()));
        let root = std::env::var_os("CARGO_MANIFEST_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
            .join("../..");
        let broadcast_dir = fs::canonicalize(output)
            .context("native deployment output directory must exist")?
            .join("foundry-broadcast");

        let endpoint = anvil.endpoint();
        let current = &checkpoint.current;
        let next = &checkpoint.next;
        let source_domain = B256::from_str(
            &std::env::var("BEEFY_SOURCE_DOMAIN").context("BEEFY_SOURCE_DOMAIN required")?,
        )?
        .0;
        let forge = Command::new("forge")
            .args([
                "script",
                "ethereum/script/BeefyLocal.s.sol:BeefyLocal",
                "--force",
                "--root",
                "ethereum",
                "--rpc-url",
                &endpoint,
                "--broadcast",
                "--slow",
                "--private-key",
                &private_key,
                "--sig",
                "run()",
            ])
            .current_dir(&root)
            .env("FOUNDRY_BROADCAST", &broadcast_dir)
            .env("PRIVATE_KEY", &private_key)
            .env(
                "LOCAL_RECOVERY_PRIVATE_KEY",
                format!("0x{}", hex::encode(anvil.keys()[1].to_bytes())),
            )
            .env("BEEFY_SOURCE_DOMAIN", bytes(source_domain))
            .env("BEEFY_BRIDGE_DOMAIN", bytes(snapshot.bridge_domain))
            .env("BEEFY_MMR_START_BLOCK", mmr_start_block.to_string())
            .env("BEEFY_INITIAL_BLOCK", initial_block.to_string())
            .env(
                "BEEFY_INITIAL_SOURCE_TIMESTAMP_MS",
                snapshot.source_timestamp_ms.to_string(),
            )
            .env("BEEFY_CURRENT_ID", current.id.to_string())
            .env("BEEFY_CURRENT_LENGTH", current.keys.len().to_string())
            .env("BEEFY_CURRENT_ROOT", bytes(current.root))
            .env("BEEFY_NEXT_ID", next.id.to_string())
            .env("BEEFY_NEXT_LENGTH", next.keys.len().to_string())
            .env("BEEFY_NEXT_ROOT", bytes(next.root))
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await
            .context("run BeefyLocal Foundry deployment")?;
        ensure!(
            forge.status.success(),
            "BeefyLocal deployment failed with status {} (stdout: {}; stderr: {})",
            forge.status,
            redact_process_output(&forge.stdout, &private_key),
            redact_process_output(&forge.stderr, &private_key)
        );

        let broadcast = broadcast_dir.join("BeefyLocal.s.sol/31337/run-latest.json");
        ensure!(
            broadcast.is_file(),
            "Foundry deployment broadcast is missing"
        );
        let broadcast_bytes = fs::read(&broadcast).context("read Foundry deployment broadcast")?;
        if output.is_dir() {
            fs::write(output.join("ethereum-broadcast.json"), &broadcast_bytes)
                .context("copy Foundry deployment broadcast")?;
        }
        let deployment: Value = serde_json::from_slice(&broadcast_bytes)
            .context("decode Foundry deployment broadcast")?;
        let returns = deployment.get("returns").unwrap_or(&deployment);
        let client_address =
            return_address(returns, "clientAddress").context("read BeefyClient named return")?;
        let verifier_address = return_address(returns, "verifierAddress")
            .context("read VaraQueueRootVerifier named return")?;
        let queue_address =
            return_address(returns, "queueAddress").context("read MessageQueue named return")?;
        let receiver_address = return_address(returns, "receiverAddress")
            .context("read MessageHandlerMock named return")?;

        let api = EthApi::new(
            &anvil.ws_endpoint(),
            &address_string(queue_address),
            Some(&private_key),
            None,
            None,
        )
        .await
        .context("connect Ethereum client to Anvil")?;
        let transactions = deployment_transactions(&deployment)?;
        for address in [
            client_address,
            verifier_address,
            queue_address,
            receiver_address,
        ] {
            let code = api
                .raw_provider()
                .get_code_at(address)
                .await
                .context("read deployed bytecode")?;
            ensure!(
                !code.is_empty(),
                "deployment returned an address without code"
            );
        }

        let ethereum = Self {
            _anvil: Some(anvil),
            owned_fixture_network: None,
            public_network: PublicEthereumNetwork::Hoodi,
            api,
            client_address,
            verifier_address,
            queue_address,
            receiver_address,
            transactions,
        };
        let deployed = ethereum.checkpoint().await?;
        ensure!(
            deployed.block == initial_block && deployed.root == [0; 32],
            "bootstrap client did not preserve initial block and zero accepted root"
        );
        ensure!(
            deployed.source_timestamp_ms == snapshot.source_timestamp_ms,
            "bootstrap source timestamp was not authenticated"
        );
        ensure!(
            deployed.current_id == current.id
                && deployed.current_len == current.keys.len() as u64
                && deployed.current_root == current.root
                && deployed.next_id == next.id
                && deployed.next_len == next.keys.len() as u64
                && deployed.next_root == next.root,
            "deployment authority checkpoint does not match trusted source checkpoint"
        );
        let client = BeefyClient::new(ethereum.client_address, ethereum.api.raw_provider().clone());
        ensure!(
            client.bridgeDomain().call().await? == B256::from(snapshot.bridge_domain)
                && client.sourceDomain().call().await? == B256::from(source_domain)
                && client.mmrStartBlock().call().await? == mmr_start_block,
            "deployed source identity does not match bootstrap"
        );
        verify_secure_policy(ethereum.client_address, ethereum.api.raw_provider().clone()).await?;
        let verifier = VaraQueueRootVerifier::new(
            ethereum.verifier_address,
            ethereum.api.raw_provider().clone(),
        );
        ensure!(
            verifier.beefyClient().call().await? == ethereum.client_address
                && verifier.messageQueue().call().await? == ethereum.queue_address
                && verifier.destinationChainId().call().await? == U256::from(31_337u64),
            "deployed adapter immutable bindings differ from MessageQueue/client/chain"
        );
        Ok(ethereum)
    }

    pub async fn connect(
        endpoint: &str,
        wallet: &Path,
        manifest: &Value,
        fixture: Option<&OwnedFixtureNetwork>,
        public_network: PublicEthereumNetwork,
    ) -> Result<Self> {
        let ethereum =
            Self::connect_addresses(endpoint, Some(wallet), manifest, fixture, public_network)
                .await?;
        verify_manifest(&ethereum, manifest).await?;
        Ok(ethereum)
    }

    /// Read the deployment only after independently approved network authentication.
    pub async fn token_manifest(
        endpoint: &str,
        wallet: &Path,
        addresses: &Value,
        fixture: Option<&OwnedFixtureNetwork>,
    ) -> Result<Value> {
        let ethereum = Self::connect_addresses(
            endpoint,
            Some(wallet),
            addresses,
            fixture,
            PublicEthereumNetwork::Hoodi,
        )
        .await?;
        verify_secure_policy(ethereum.client_address, ethereum.api.raw_provider().clone()).await?;
        verify_bindings(
            ethereum.verifier_address,
            ethereum.client_address,
            ethereum.queue_address,
            ethereum.expected_chain_id(),
            ethereum.api.raw_provider().clone(),
        )
        .await?;
        let manifest = ethereum.manifest().await?;
        verify_manifest(&ethereum, &manifest).await?;
        Ok(manifest)
    }

    async fn connect_addresses(
        endpoint: &str,
        wallet: Option<&Path>,
        manifest: &Value,
        fixture: Option<&OwnedFixtureNetwork>,
        public_network: PublicEthereumNetwork,
    ) -> Result<Self> {
        ensure!(
            fixture.is_none() || public_network == PublicEthereumNetwork::Hoodi,
            "mainnet cannot use an owned fixture; HOLD"
        );
        ensure!(
            public_network != PublicEthereumNetwork::Mainnet
                || endpoint.starts_with("https://")
                || endpoint.starts_with("wss://"),
            "mainnet requires HTTPS or WSS; HOLD"
        );
        let chain_id = fixture.map_or(public_network.chain_id(), |network| {
            network.ethereum_chain_id
        });
        ensure!(
            manifest_u64(manifest, "chainId")? == chain_id,
            "deployment chain id differs from approved network; HOLD"
        );
        if let Some(network) = fixture {
            network.validate_endpoint(endpoint)?;
        }
        let client_address = manifest_address(manifest, "client")?;
        let verifier_address = manifest_address(manifest, "verifier")?;
        let queue_address = manifest_address(manifest, "queue")?;
        let receiver_address = manifest_address(manifest, "receiver")?;
        let endpoint = hoodi_rpc_endpoint(endpoint)?;
        if let Some(network) = fixture {
            ensure_ethereum_network(
                &endpoint,
                network.ethereum_chain_id,
                network.ethereum_genesis_hash,
            )
            .await?;
        } else {
            ensure_ethereum_network(
                &endpoint,
                public_network.chain_id(),
                public_network.genesis(),
            )
            .await?;
        }
        let private_key = wallet
            .map(read_hoodi_wallet)
            .transpose()?
            .map(|(_, key)| key);
        let api = EthApi::new(
            &endpoint,
            &address_string(queue_address),
            private_key.as_deref(),
            None,
            None,
        )
        .await
        .context("connect Ethereum deployment")?;
        Ok(Self {
            _anvil: None,
            owned_fixture_network: fixture.cloned(),
            public_network,
            api,
            client_address,
            verifier_address,
            queue_address,
            receiver_address,
            transactions: Vec::new(),
        })
    }

    pub(crate) async fn connect_readonly(
        endpoint: &str,
        manifest: &Value,
        fixture: Option<&OwnedFixtureNetwork>,
        public_network: PublicEthereumNetwork,
    ) -> Result<Self> {
        let ethereum =
            Self::connect_addresses(endpoint, None, manifest, fixture, public_network).await?;
        verify_manifest(&ethereum, manifest).await?;
        Ok(ethereum)
    }

    fn expected_chain_id(&self) -> u64 {
        self.owned_fixture_network
            .as_ref()
            .map_or(self.public_network.chain_id(), |network| {
                network.ethereum_chain_id
            })
    }

    pub(crate) async fn finality_genesis(&self) -> Result<B256> {
        if let Some(network) = &self.owned_fixture_network {
            return Ok(network.ethereum_genesis_hash);
        }
        // The synthetic Anvil rehearsal exists only inside the pre-existing unit fixtures.
        #[cfg(test)]
        if self._anvil.is_some() {
            let genesis = self
                .api
                .raw_provider()
                .get_block_by_number(0u64.into())
                .await?
                .context("owned rehearsal genesis is missing")?;
            ensure!(
                genesis.header.number == 0
                    && genesis.header.inner.hash_slow() == genesis.header.hash,
                "owned rehearsal genesis is not canonical RLP; HOLD"
            );
            return Ok(genesis.header.hash);
        }
        Ok(self.public_network.genesis())
    }

    pub(crate) async fn queue_root_minimum(&self) -> Result<u64> {
        Ok(
            TokenQueueBinding::new(self.queue_address, self.api.raw_provider().clone())
                .beefyRootMinimum()
                .call()
                .await?
                .try_into()?,
        )
    }

    /// Recover immutable constructor evidence, not the advancing latest checkpoint.
    pub(crate) async fn authenticate_bootstrap(
        &self,
        directory: &Path,
        saved: &Value,
        deployment_transaction: Option<B256>,
    ) -> Result<(DestinationCheckpoint, Value)> {
        use alloy::sol_types::SolValue;
        if saved.is_object() {
            ensure!(
                deployment_transaction.is_none_or(|hint| saved["txHash"]
                    .as_str()
                    .and_then(|hash| hash.parse::<B256>().ok())
                    == Some(hint)),
                "client deployment transaction hint conflicts with saved bootstrap; HOLD"
            );
        }
        let mut view = self
            .api
            .verified_finalized_view_with_archive(
                &directory.join("headers"),
                self.finality_genesis().await?,
            )
            .await?;
        let (number, hash, transaction_hash) = if saved.is_object() {
            (
                manifest_u64(saved, "block")?,
                manifest_b256(saved, "blockHash")?,
                manifest_b256(saved, "txHash")?,
            )
        } else if let Some(transaction_hash) = deployment_transaction {
            // Receipt JSON is only a locator. The finalized header and both tries authenticate it below.
            let receipt = self
                .api
                .raw_provider()
                .get_transaction_receipt(transaction_hash)
                .await?
                .context("client deployment transaction receipt unavailable; HOLD")?;
            ensure!(
                receipt.transaction_hash == transaction_hash,
                "client deployment locator transaction changed; HOLD"
            );
            (
                receipt
                    .block_number
                    .context("client deployment locator block unavailable; HOLD")?,
                receipt
                    .block_hash
                    .context("client deployment locator hash unavailable; HOLD")?,
                transaction_hash,
            )
        } else {
            // Code existence only locates a candidate; the constructor is authenticated below.
            let mut low = 0;
            let mut high = view.block_number();
            while low < high {
                let middle = low + (high - low) / 2;
                let block = self
                    .api
                    .raw_provider()
                    .get_block_by_number(BlockNumberOrTag::Number(middle))
                    .await?
                    .context("deployment search header unavailable; HOLD")?;
                let code = self
                    .api
                    .raw_provider()
                    .get_code_at(self.client_address)
                    .block_id(BlockId::hash_canonical(block.header.hash))
                    .await?;
                if code.is_empty() {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            let block = self
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Number(low))
                .full()
                .await?
                .context("deployment full block unavailable; HOLD")?;
            let transactions = block
                .transactions
                .try_into_transactions()
                .map_err(|_| anyhow::anyhow!("deployment transactions missing; HOLD"))?;
            let mut deployment = None;
            for transaction in transactions {
                let signed = transaction.into_inner();
                if signed.to().is_none()
                    && signed.recover_signer()?.create(signed.nonce()) == self.client_address
                {
                    ensure!(
                        deployment
                            .replace(B256::from(keccak256(&signed.encoded_2718())))
                            .is_none(),
                        "multiple client constructors; HOLD"
                    );
                }
            }
            (
                low,
                block.header.hash,
                deployment.context("client creation transaction unavailable; HOLD")?,
            )
        };
        view.verify_blocks(
            self.api.raw_provider(),
            &std::collections::BTreeMap::from([(number, hash)]),
        )
        .await?;
        let proof = authenticate_transaction_receipt(
            self,
            &view,
            number,
            hash,
            transaction_hash,
            &directory.join("receipts"),
        )
        .await?;
        let mut encoded = proof.raw_transaction.as_slice();
        let signed = TxEnvelope::decode_2718(&mut encoded)?;
        ensure!(
            encoded.is_empty()
                && signed.to().is_none()
                && signed.chain_id() == Some(self.api.raw_provider().get_chain_id().await?)
                && signed.recover_signer()?.create(signed.nonce()) == self.client_address
                && proof.receipt.status(),
            "client creation signed identity changed; HOLD"
        );
        let input = signed.input().as_ref();
        let code = BeefyClient::BYTECODE.as_ref();
        ensure!(
            input.starts_with(code),
            "client constructor does not match approved implementation; HOLD"
        );
        type Constructor = (
            B256,
            U256,
            Address,
            u64,
            u64,
            u64,
            (u128, u128, B256),
            (u128, u128, B256),
        );
        let (source_domain, chain, queue, start, block, timestamp, current, next) =
            Constructor::abi_decode(&input[code.len()..])?;
        let client = BeefyClient::new(self.client_address, self.api.raw_provider().clone());
        ensure!(
            source_domain == client.sourceDomain().call().await?
                && chain == U256::from(self.api.raw_provider().get_chain_id().await?)
                && queue == self.queue_address
                && start == client.mmrStartBlock().call().await?,
            "immutable constructor deployment domain changed; HOLD"
        );
        Ok((
            DestinationCheckpoint {
                block,
                root: [0; 32],
                source_timestamp_ms: timestamp,
                current_id: current.0.try_into()?,
                current_len: current.1.try_into()?,
                current_root: current.2.into(),
                next_id: next.0.try_into()?,
                next_len: next.1.try_into()?,
                next_root: next.2.into(),
            },
            json!({"block":number,"blockHash":bytes(hash),"txHash":bytes(transaction_hash)}),
        ))
    }

    pub(crate) async fn queue_max_block_number(&self) -> Result<u64> {
        let provider = self.api.raw_provider().clone();
        let finalized = provider
            .get_block_by_number(BlockNumberOrTag::Finalized)
            .await?
            .context("finalized queue-progress block is missing")?;
        let value = TokenQueueBinding::new(self.queue_address, provider)
            .maxBlockNumber()
            .block(BlockId::hash(finalized.header.hash))
            .call()
            .await?;
        value.try_into().context("queue maxBlockNumber exceeds u64")
    }

    pub async fn verify_token_bindings(&self, gear_manager: [u8; 32]) -> Result<()> {
        ensure!(
            self.queue_address != Address::from_str("0xab51a342680b774137a8231cbcebb50417ab9aeb")?,
            "token client cannot publish to the existing message-only queue"
        );
        let provider = self.api.raw_provider().clone();
        let manager = TokenManagerBinding::new(self.receiver_address, provider.clone());
        ensure!(
            manager.messageQueue().call().await? == self.queue_address,
            "token manager does not belong to the isolated queue"
        );
        ensure!(
            TokenQueueBinding::new(self.queue_address, provider)
                .verifier()
                .call()
                .await?
                == self.verifier_address,
            "queue verifier does not belong to the isolated BEEFY client"
        );
        ensure!(
            manager
                .vftManagers()
                .call()
                .await?
                .contains(&B256::from(gear_manager)),
            "Gear token manager is not registered by the EVM manager"
        );
        Ok(())
    }

    #[cfg(test)]
    pub fn receiver_address(&self) -> [u8; 20] {
        self.receiver_address.into()
    }

    pub async fn recovery_configuration(&self) -> Result<Value> {
        let provider = self.api.raw_provider().clone();
        let address = TokenQueueBinding::new(self.queue_address, provider.clone())
            .beefyRecoveryTimelock()
            .call()
            .await?;
        let code = provider.get_code_at(address).await?;
        ensure!(
            address != Address::ZERO && !code.is_empty(),
            "BEEFY recovery timelock has no code"
        );
        let delay = RecoveryTimelockBinding::new(address, provider)
            .getMinDelay()
            .call()
            .await?;
        ensure!(
            delay > U256::ZERO,
            "BEEFY recovery timelock must retain a positive delay"
        );
        Ok(json!({
            "address": address_string(address),
            "codeHash": format!("0x{}", hex::encode(keccak256(code.as_ref()))),
            "minDelay": delay.to_string(),
        }))
    }

    pub async fn manifest(&self) -> Result<Value> {
        let mut bytecode_hashes = serde_json::Map::new();
        for (name, address) in [
            ("client", self.client_address),
            ("verifier", self.verifier_address),
            ("queue", self.queue_address),
            ("receiver", self.receiver_address),
        ] {
            let code = self
                .api
                .raw_provider()
                .get_code_at(address)
                .await
                .with_context(|| format!("read {name} bytecode"))?;
            ensure!(!code.is_empty(), "{name} has no deployed bytecode");
            bytecode_hashes.insert(
                name.to_owned(),
                Value::String(format!("0x{}", hex::encode(keccak256(code.as_ref())))),
            );
        }
        let checkpoint = self.checkpoint().await?;
        let client = BeefyClient::new(self.client_address, self.api.raw_provider().clone());
        let verifier =
            VaraQueueRootVerifier::new(self.verifier_address, self.api.raw_provider().clone());
        let source_domain = client.sourceDomain().call().await?;
        let mmr_start_block = client.mmrStartBlock().call().await?;
        let policy = json!({
            "randaoCommitDelay": client.randaoCommitDelay().call().await?.to_string(),
            "randaoCommitExpiration": client.randaoCommitExpiration().call().await?.to_string(),
            "minNumRequiredSignatures": client.minNumRequiredSignatures().call().await?.to_string(),
            "fiatShamirRequiredSignatures": client.fiatShamirRequiredSignatures().call().await?.to_string(),
            "maxValidators": client.MAX_VALIDATORS().call().await?.to_string(),
            "maxSourceAgeMs": client.MAX_SOURCE_AGE_MS().call().await?.to_string(),
            "maxFutureSourceSkewMs": client.MAX_FUTURE_SOURCE_SKEW_MS().call().await?.to_string(),
            "isLive": client.isLive().call().await?,
        });
        let chain_id = self
            .api
            .raw_provider()
            .get_chain_id()
            .await
            .context("read Ethereum chain id")?;
        Ok(json!({
            "chainId": chain_id,
            "client": address_string(self.client_address),
            "verifier": address_string(self.verifier_address),
            "queue": address_string(self.queue_address),
            "receiver": address_string(self.receiver_address),
            "recovery": self.recovery_configuration().await?,
            "bindings": {
                "beefyClient": address_string(verifier.beefyClient().call().await?),
                "messageQueue": address_string(verifier.messageQueue().call().await?),
                "destinationChainId": verifier.destinationChainId().call().await?.to_string(),
            },
            "sourceDomain": bytes(source_domain),
            "bridgeDomain": bytes(client.bridgeDomain().call().await?),
            "mmrStartBlock": mmr_start_block,
            "checkpoint": {
                "block": checkpoint.block,
                "root": bytes(checkpoint.root),
                "sourceTimestampMs": checkpoint.source_timestamp_ms,
                "currentId": checkpoint.current_id,
                "currentLength": checkpoint.current_len,
                "currentRoot": bytes(checkpoint.current_root),
                "nextId": checkpoint.next_id,
                "nextLength": checkpoint.next_len,
                "nextRoot": bytes(checkpoint.next_root),
            },
            "policy": policy,
            "bytecodeHashes": bytecode_hashes,
        }))
    }

    pub async fn checkpoint(&self) -> Result<DestinationCheckpoint> {
        let mut retries = 0;
        loop {
            let latest = self
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Latest)
                .await?
                .context("latest Ethereum block is missing")?;
            match self.checkpoint_at(BlockId::hash(latest.header.hash)).await {
                Ok(checkpoint) => return Ok(checkpoint),
                Err(err)
                    if retries < 4 && format!("{err:#}").contains("header for hash not found") =>
                {
                    retries += 1;
                    sleep(Duration::from_secs(1)).await;
                }
                Err(err) => return Err(err),
            }
        }
    }

    pub(crate) async fn checkpoint_at(&self, block: BlockId) -> Result<DestinationCheckpoint> {
        let client = BeefyClient::new(self.client_address, self.api.raw_provider().clone());
        let current = client
            .currentValidatorSet()
            .block(block)
            .call()
            .await
            .context("read current BEEFY set")?;
        let next = client
            .nextValidatorSet()
            .block(block)
            .call()
            .await
            .context("read next BEEFY set")?;
        Ok(DestinationCheckpoint {
            block: client
                .latestBeefyBlock()
                .block(block)
                .call()
                .await
                .context("read latest BEEFY block")?,
            root: client
                .latestMMRRoot()
                .block(block)
                .call()
                .await
                .context("read latest MMR root")?
                .into(),
            source_timestamp_ms: client
                .lastAuthenticatedSourceTimestampMs()
                .block(block)
                .call()
                .await
                .context("read authenticated source timestamp")?,
            current_id: u64::try_from(current.id).context("current BEEFY set id overflows u64")?,
            current_len: u64::try_from(current.length)
                .context("current BEEFY set length overflows u64")?,
            current_root: current.root.into(),
            next_id: u64::try_from(next.id).context("next BEEFY set id overflows u64")?,
            next_len: u64::try_from(next.length).context("next BEEFY set length overflows u64")?,
            next_root: next.root.into(),
        })
    }

    pub async fn verify_accepted_commitment(
        &self,
        tx_hash: B256,
        anchor: &CapturedCommitment,
        snapshot: &QueueSnapshot,
    ) -> Result<(u64, B256)> {
        use alloy::sol_types::{SolCall, SolValue};
        let receipt = self
            .api
            .raw_provider()
            .get_transaction_receipt(tx_hash)
            .await?
            .context("accepted commitment receipt is unavailable")?;
        ensure!(
            receipt.transaction_hash == tx_hash
                && receipt.status()
                && receipt.to == Some(self.client_address),
            "accepted commitment receipt identity or status is invalid"
        );
        let number = receipt
            .block_number
            .context("accepted receipt has no block number")?;
        let hash = receipt
            .block_hash
            .context("accepted receipt has no block hash")?;
        wait_canonical_receipt(self.api.raw_provider(), number, hash).await?;
        let signed = self
            .api
            .raw_provider()
            .get_transaction_by_hash(tx_hash)
            .await?
            .context("accepted signed transaction unavailable; HOLD")?
            .into_inner();
        let raw = signed.encoded_2718();
        let identity =
            signed_transaction_identity(&raw, tx_hash, signed.nonce(), self.client_address)?;
        ensure!(
            signed.chain_id() == Some(self.api.raw_provider().get_chain_id().await?)
                && identity.from == receipt.from,
            "accepted signed transaction changed sender or chain; HOLD"
        );
        validate_recorded_commitment_transaction(&raw, &anchor.raw)?;
        let call = BeefyClient::submitFiatShamirCall::abi_decode(signed.input())?;
        ensure!(
            call.snapshot.abi_encode() == to_sol_snapshot(snapshot).abi_encode()
                && call.leaf.parentNumber.checked_add(1) == Some(anchor.block)
                && call.leaf.nextAuthoritySetID == anchor.next.id
                && call.leaf.nextAuthoritySetLen == anchor.next.keys.len() as u32
                && call.leaf.nextAuthoritySetRoot == B256::from(anchor.next.root)
                && call.leaf.parachainHeadsRoot == B256::from(snapshot.hash()),
            "accepted signed freshness or authority transition changed; HOLD"
        );
        let event = B256::from(keccak256(b"NewMMRRoot(bytes32,uint64)"));
        ensure!(
            receipt.as_ref().logs().iter().any(|log| {
                let data = log.data().data.as_ref();
                log.address() == self.client_address
                    && log.topic0() == Some(&event)
                    && log.topics().len() == 1
                    && data.len() == 64
                    && data[..32] == anchor.validated.mmr_root
                    && U256::from_be_slice(&data[32..]) == U256::from(anchor.block)
            }),
            "accepted receipt does not authenticate source commitment"
        );
        let latest = self.checkpoint().await?;
        // Public Hoodi RPC prunes historical state; the unchanged latest checkpoint is equivalent.
        let checkpoint = if latest.block == u64::from(anchor.block) {
            latest
        } else {
            self.checkpoint_at(BlockId::hash(hash)).await?
        };
        ensure!(
            checkpoint.block >= u64::from(anchor.block),
            "accepted checkpoint regressed behind original source block; HOLD"
        );
        // Several permissionless updates may share one Ethereum block. Their exact
        // signed calldata and original receipt effects, not end-of-block state, own each outcome.
        if checkpoint.block > u64::from(anchor.block) {
            return Ok((number, hash));
        }
        ensure!(
            checkpoint.block == u64::from(anchor.block)
                && checkpoint.root == anchor.validated.mmr_root
                && checkpoint.source_timestamp_ms == snapshot.source_timestamp_ms
                && checkpoint.current_id == anchor.current.id
                && checkpoint.current_len == anchor.current.keys.len() as u64
                && checkpoint.current_root == anchor.current.root
                && checkpoint.next_id == anchor.next.id
                && checkpoint.next_len == anchor.next.keys.len() as u64
                && checkpoint.next_root == anchor.next.root,
            "accepted receipt checkpoint disagrees with authenticated source history"
        );
        Ok((number, hash))
    }

    pub async fn submit_commitment(
        &mut self,
        anchor: &CapturedCommitment,
        freshness_leaf: &RuntimeLeaf,
        snapshot: &QueueSnapshot,
        freshness: &SimplifiedMmrProof,
        prior: Option<&Value>,
        mut journal: impl FnMut(&Value, Option<&alloy::rpc::types::TransactionReceipt>) -> Result<()>,
    ) -> Result<()> {
        let validated = &anchor.validated;
        let keys = &anchor.current.keys;
        let before = self.checkpoint().await?;
        let set_id = validated.signed.commitment.validator_set_id;
        ensure!(
            set_id == before.current_id || set_id == before.next_id,
            "commitment authority set {set_id} is not current or next"
        );
        if set_id == before.next_id {
            ensure!(
                freshness_leaf.beefy_next_authority_set.id > before.next_id,
                "next-set commitment does not carry a later authority set"
            );
        }
        ensure!(
            snapshot.hash() == freshness_leaf.leaf_extra
                && freshness_leaf.parent_number_and_hash.0 + 1
                    == validated.signed.commitment.block_number,
            "freshness proof does not authenticate the newest commitment leaf"
        );

        let wire_keys: Vec<[u8; 33]> = keys
            .iter()
            .map(|key| {
                ensure!(key.len() == 33, "BEEFY authority key must be 33 bytes");
                Ok(key.as_slice().try_into().expect("checked key length"))
            })
            .collect::<Result<_>>()?;
        ensure!(
            validated
                .signed_indices
                .iter()
                .all(|index| usize::try_from(*index).is_ok_and(|i| i < wire_keys.len())),
            "signed authority position is outside the authority set"
        );
        ensure!(
            wire_keys.len() <= 256,
            "BEEFY authority set exceeds uint256 bitfield"
        );
        let mut bitfield = vec![U256::ZERO];
        let available: BTreeSet<usize> = validated
            .signed_indices
            .iter()
            .map(|index| usize::try_from(*index).expect("checked above"))
            .collect();
        for index in &available {
            bitfield[index / 256] |= U256::from(1u8) << (index % 256);
        }

        let commitment = BeefyClient::Commitment {
            blockNumber: validated.signed.commitment.block_number,
            validatorSetID: validated.signed.commitment.validator_set_id,
            payload: commitment_payload(&validated.signed.commitment.payload)?,
        };
        let client = BeefyClient::new(self.client_address, self.api.raw_provider().clone());
        let selected_words = client
            .createFiatShamirFinalBitfield(commitment.clone(), bitfield.clone())
            .call()
            .await
            .context("create Fiat-Shamir final bitfield")?;
        let mut selected = Vec::new();
        for (word_index, word) in selected_words.iter().enumerate() {
            for bit in 0..256 {
                if word.bit(bit) {
                    let index = word_index * 256 + bit;
                    ensure!(
                        available.contains(&index),
                        "Fiat-Shamir selected an unsigned position"
                    );
                    selected.push(index);
                }
            }
        }
        ensure!(!selected.is_empty(), "Fiat-Shamir selected no signatures");

        let proofs = selected
            .into_iter()
            .map(|index| {
                let signature = validated
                    .signed
                    .signatures
                    .get(index)
                    .and_then(Option::as_ref)
                    .context("selected authority signature is absent")?;
                let raw: &[u8] = signature.as_ref();
                ensure!(
                    raw.len() == 65 && raw[64] <= 1,
                    "unsupported BEEFY ECDSA recovery id"
                );
                let authority = authority_proof(&wire_keys, index)?;
                let mut r = [0; 32];
                let mut s = [0; 32];
                r.copy_from_slice(&raw[..32]);
                s.copy_from_slice(&raw[32..64]);
                Ok(BeefyClient::ValidatorProof {
                    v: raw[64] + 27,
                    r: B256::from(r),
                    s: B256::from(s),
                    index: U256::from(index),
                    account: Address::from(authority.address),
                    proof: authority.siblings.into_iter().map(B256::from).collect(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let source_timestamp_ms = snapshot.source_timestamp_ms;
        let leaf = to_sol_leaf(freshness_leaf)?;
        let sol_snapshot = to_sol_snapshot(snapshot);
        let items: Vec<B256> = freshness.items.iter().copied().map(B256::from).collect();
        let order = U256::from_be_bytes(freshness.proof_order);
        let call = client.submitFiatShamir(
            commitment,
            bitfield,
            proofs,
            leaf,
            sol_snapshot,
            items,
            order,
        );
        let provider = self.api.raw_provider();
        let sender = provider.default_signer_address();
        let latest = provider.get_transaction_count(sender).await?;
        let pending = provider.get_transaction_count(sender).pending().await?;
        let mut transaction = prior
            .cloned()
            .unwrap_or_else(|| json!({"block": anchor.block, "rawScale": bytes(&anchor.raw)}));
        let nonce = match transaction.get("nonce") {
            Some(nonce) => nonce.as_u64().context("invalid saved commitment nonce")?,
            None => {
                ensure!(latest == pending, "follower has an outstanding transaction");
                transaction["nonce"] = json!(pending);
                journal(&transaction, None)?;
                pending
            }
        };
        ensure!(
            latest == nonce && pending == nonce,
            "commitment nonce is uncertain; refusing a replacement nonce"
        );
        let filled = provider
            .fill(call.nonce(nonce).into_transaction_request())
            .await
            .context("sign original Fiat-Shamir commitment")?;
        let raw = filled
            .as_envelope()
            .context("commitment transaction was not signed")?
            .encoded_2718();
        transaction["rawTransaction"] = json!(bytes(&raw));
        transaction["txHash"] = json!(bytes(keccak256(&raw)));
        transaction["sender"] = json!(format!("{sender:#x}"));
        transaction["chainId"] = json!(filled
            .as_envelope()
            .context("signed commitment envelope missing")?
            .chain_id());
        transaction["clientAddress"] = json!(format!("{:#x}", self.client_address));
        journal(&transaction, None)?;
        let receipt = self.resume_commitment(&transaction).await?;
        journal(&transaction, Some(&receipt))?;
        ensure!(
            receipt.status(),
            "Fiat-Shamir commitment transaction reverted"
        );
        self.transactions
            .push(receipt_value("submitFiatShamir", &receipt, None));

        let receipt_block = receipt
            .block_hash
            .context("accepted receipt has no block hash")?;
        let latest = self.checkpoint().await?;
        let after = if latest.block == u64::from(validated.signed.commitment.block_number) {
            latest
        } else {
            self.checkpoint_at(BlockId::hash(receipt_block)).await?
        };
        ensure!(
            after.block == u64::from(validated.signed.commitment.block_number)
                && after.root == validated.mmr_root,
            "accepted BEEFY checkpoint does not match commitment"
        );
        ensure!(
            after.source_timestamp_ms == source_timestamp_ms,
            "accepted source timestamp does not match authenticated freshness metadata"
        );
        if set_id == before.next_id {
            ensure!(
                after.current_id == before.next_id
                    && after.next_id == freshness_leaf.beefy_next_authority_set.id
                    && after.next_root == freshness_leaf.beefy_next_authority_set.keyset_commitment,
                "accepted next-set transition does not match freshness leaf"
            );
        } else {
            ensure!(
                after.current_id == before.current_id
                    && after.next_id == before.next_id
                    && after.current_root == before.current_root
                    && after.next_root == before.next_root,
                "current-set commitment unexpectedly changed authority state"
            );
        }
        Ok(())
    }
    pub async fn resume_commitment(
        &self,
        transaction: &Value,
    ) -> Result<alloy::rpc::types::TransactionReceipt> {
        let raw = hex::decode(
            transaction["rawTransaction"]
                .as_str()
                .context("saved signed commitment missing")?
                .trim_start_matches("0x"),
        )?;
        let hash = B256::from_str(
            transaction["txHash"]
                .as_str()
                .context("saved commitment hash missing")?,
        )?;
        let nonce = transaction["nonce"]
            .as_u64()
            .context("saved commitment nonce missing")?;
        let provider = self.api.raw_provider();
        let signed = signed_transaction_identity(&raw, hash, nonce, self.client_address)?;
        let mut encoded = raw.as_slice();
        ensure!(
            TxEnvelope::decode_2718(&mut encoded)?.chain_id()
                == Some(provider.get_chain_id().await?),
            "saved commitment belongs to another Ethereum chain; HOLD"
        );
        ensure!(
            signed.from == provider.default_signer_address(),
            "saved commitment signer differs from follower wallet"
        );
        await_original_commitment(provider, signed, &raw).await
    }
}

fn hoodi_rpc_endpoint(endpoint: &str) -> Result<String> {
    let endpoint = if endpoint.is_empty() {
        HOODI_DEFAULT_RPC
    } else {
        endpoint
    };
    ensure!(
        matches!(
            url::Url::parse(endpoint)?.scheme(),
            "http" | "https" | "ws" | "wss"
        ),
        "Hoodi Ethereum endpoint must use HTTP(S) or WS(S)"
    );
    Ok(endpoint.to_owned())
}

fn read_hoodi_wallet(path: &Path) -> Result<(Address, String)> {
    ensure!(path.is_absolute(), "Hoodi wallet path must be absolute");
    let metadata = fs::metadata(path).context("read Hoodi wallet metadata")?;
    #[cfg(unix)]
    ensure!(
        metadata.permissions().mode() & 0o777 == 0o600,
        "Hoodi wallet permissions must be 0600"
    );
    let value: Value = serde_json::from_slice(&fs::read(path).context("read Hoodi wallet JSON")?)
        .context("decode Hoodi wallet JSON")?;
    let address_text = value
        .get("address")
        .and_then(Value::as_str)
        .context("Hoodi wallet is missing address")?;
    let private_key = value
        .get("private_key")
        .and_then(Value::as_str)
        .context("Hoodi wallet is missing private_key")?
        .to_owned();
    let address = Address::from_str(address_text)
        .map_err(|_| anyhow::anyhow!("Hoodi wallet address is invalid"))?;
    let key_text = private_key.strip_prefix("0x").unwrap_or(&private_key);
    ensure!(
        key_text.len() == 64,
        "Hoodi wallet private_key must be 32-byte hex"
    );
    let key_bytes = hex::decode(key_text)
        .map_err(|_| anyhow::anyhow!("Hoodi wallet private_key must be 32-byte hex"))?;
    let signer = PrivateKeySigner::from_bytes(&B256::from_slice(&key_bytes))
        .map_err(|_| anyhow::anyhow!("Hoodi wallet private_key is invalid"))?;
    ensure!(
        signer.address() == address,
        "Hoodi wallet address does not match private_key"
    );
    Ok((address, private_key))
}

pub(crate) fn hoodi_wallet_address(path: &Path) -> Result<Address> {
    Ok(read_hoodi_wallet(path)?.0)
}

pub(crate) async fn wait_canonical_receipt(
    provider: &impl Provider,
    block: u64,
    receipt_hash: B256,
) -> Result<()> {
    timeout(Duration::from_secs(120), async {
        loop {
            if let Some(canonical) = provider
                .get_block_by_number(BlockNumberOrTag::Number(block))
                .await?
            {
                ensure!(
                    canonical.header.hash == receipt_hash,
                    "accepted receipt block is not canonical"
                );
                return Ok(());
            }
            sleep(Duration::from_secs(2)).await;
        }
    })
    .await
    .context("HOLD: original receipt block header remains unavailable after 120 seconds")?
}

async fn ensure_ethereum_network(
    endpoint: &str,
    chain_id: u64,
    expected_genesis: B256,
) -> Result<()> {
    let provider = PollingEthApi::new(endpoint)
        .await
        .context("connect approved Ethereum endpoint")?;
    verify_ethereum_network(&*provider, chain_id, expected_genesis).await
}

async fn verify_ethereum_network(
    provider: &impl Provider,
    chain_id: u64,
    expected_genesis: B256,
) -> Result<()> {
    ensure!(
        provider.get_chain_id().await? == chain_id,
        "Ethereum endpoint has unexpected chain id; HOLD"
    );
    let genesis = provider
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await?
        .context("Ethereum genesis block is missing")?;
    ensure!(
        genesis.header.number == 0
            && genesis.header.hash == expected_genesis
            && genesis.header.inner.hash_slow() == expected_genesis,
        "Ethereum endpoint has unexpected or noncanonical genesis block; HOLD"
    );
    Ok(())
}

fn manifest_u64(manifest: &Value, field: &str) -> Result<u64> {
    let value = manifest
        .get(field)
        .with_context(|| format!("manifest is missing {field}"))?;
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }
    value
        .as_str()
        .with_context(|| format!("manifest field {field} is not an integer"))?
        .parse()
        .with_context(|| format!("manifest field {field} is not an integer"))
}

fn manifest_u256(manifest: &Value, field: &str) -> Result<U256> {
    let value = manifest
        .get(field)
        .with_context(|| format!("manifest is missing {field}"))?;
    if let Some(value) = value.as_u64() {
        return Ok(U256::from(value));
    }
    U256::from_str(
        value
            .as_str()
            .with_context(|| format!("manifest field {field} is not an integer"))?,
    )
    .with_context(|| format!("manifest field {field} is not an integer"))
}

fn manifest_address(manifest: &Value, field: &str) -> Result<Address> {
    manifest
        .get(field)
        .and_then(parse_address)
        .with_context(|| format!("manifest is missing valid {field} address"))
}

fn manifest_b256(manifest: &Value, field: &str) -> Result<B256> {
    let value = manifest
        .get(field)
        .and_then(Value::as_str)
        .with_context(|| format!("manifest is missing {field}"))?;
    B256::from_str(value).with_context(|| format!("manifest field {field} is not bytes32"))
}

async fn verify_secure_policy<P>(client_address: Address, provider: P) -> Result<()>
where
    P: Provider + Clone,
{
    let client = BeefyClient::new(client_address, provider);
    ensure!(
        client.randaoCommitDelay().call().await? == U256::from(128u64)
            && client.randaoCommitExpiration().call().await? == U256::from(24u64)
            && client.minNumRequiredSignatures().call().await? == U256::from(86u64)
            && client.fiatShamirRequiredSignatures().call().await? == U256::from(86u64)
            && client.MAX_VALIDATORS().call().await? == U256::from(256u64)
            && client.MAX_SOURCE_AGE_MS().call().await? == U256::from(86_400_000u64)
            && client.MAX_FUTURE_SOURCE_SKEW_MS().call().await? == U256::from(120_000u64),
        "deployed BEEFY policy constants differ from approved 86/86/256 policy"
    );
    Ok(())
}

async fn verify_bindings<P>(
    verifier_address: Address,
    client_address: Address,
    queue_address: Address,
    chain_id: u64,
    provider: P,
) -> Result<()>
where
    P: Provider + Clone,
{
    let verifier = VaraQueueRootVerifier::new(verifier_address, provider.clone());
    let client = BeefyClient::new(client_address, provider);
    let source_domain = client.sourceDomain().call().await?;
    let expected_domain = crate::bridge_domain(
        source_domain.0,
        chain_id,
        queue_address
            .as_slice()
            .try_into()
            .expect("address is 20 bytes"),
    )?;
    ensure!(
        client.bridgeDomain().call().await? == B256::from(expected_domain)
            && client.destinationChainId().call().await? == U256::from(chain_id)
            && client.destinationQueue().call().await? == queue_address,
        "client domain or destination differs from source lane"
    );
    ensure!(
        verifier.beefyClient().call().await? == client_address
            && verifier.messageQueue().call().await? == queue_address
            && verifier.destinationChainId().call().await? == U256::from(chain_id),
        "deployed adapter immutable bindings differ from MessageQueue/client/chain"
    );
    Ok(())
}

async fn verify_manifest(ethereum: &Ethereum, manifest: &Value) -> Result<()> {
    let provider = ethereum.api.raw_provider().clone();
    ensure!(
        provider.get_chain_id().await? == ethereum.expected_chain_id(),
        "connected Ethereum endpoint has unexpected approved chain id"
    );
    ensure!(
        manifest["recovery"] == ethereum.recovery_configuration().await?,
        "BEEFY recovery timelock address, code or delay differs from the saved manifest"
    );
    let bridge_domain = manifest_b256(manifest, "bridgeDomain")?;
    let source_domain = manifest_b256(manifest, "sourceDomain")?;
    let mmr_start_block = manifest_u64(manifest, "mmrStartBlock")?;
    let client = BeefyClient::new(ethereum.client_address, provider.clone());
    ensure!(
        client.bridgeDomain().call().await? == bridge_domain
            && client.sourceDomain().call().await? == source_domain
            && client.mmrStartBlock().call().await? == mmr_start_block,
        "Hoodi source identity or MMR start differs from manifest"
    );
    verify_secure_policy(ethereum.client_address, provider.clone()).await?;
    let policy = manifest
        .get("policy")
        .context("manifest is missing policy")?;
    for (name, actual) in [
        (
            "randaoCommitDelay",
            client.randaoCommitDelay().call().await?,
        ),
        (
            "randaoCommitExpiration",
            client.randaoCommitExpiration().call().await?,
        ),
        (
            "minNumRequiredSignatures",
            client.minNumRequiredSignatures().call().await?,
        ),
        (
            "fiatShamirRequiredSignatures",
            client.fiatShamirRequiredSignatures().call().await?,
        ),
        ("maxValidators", client.MAX_VALIDATORS().call().await?),
        ("maxSourceAgeMs", client.MAX_SOURCE_AGE_MS().call().await?),
        (
            "maxFutureSourceSkewMs",
            client.MAX_FUTURE_SOURCE_SKEW_MS().call().await?,
        ),
    ] {
        ensure!(
            manifest_u256(policy, name)? == actual,
            "Hoodi policy differs from saved manifest"
        );
    }
    verify_bindings(
        ethereum.verifier_address,
        ethereum.client_address,
        ethereum.queue_address,
        ethereum.expected_chain_id(),
        provider.clone(),
    )
    .await?;
    let bindings = manifest
        .get("bindings")
        .context("manifest is missing bindings")?;
    let verifier = VaraQueueRootVerifier::new(ethereum.verifier_address, provider.clone());
    ensure!(
        manifest_address(bindings, "beefyClient")? == verifier.beefyClient().call().await?
            && manifest_address(bindings, "messageQueue")?
                == verifier.messageQueue().call().await?
            && manifest_u64(bindings, "destinationChainId")?
                == u64::try_from(verifier.destinationChainId().call().await?)?,
        "Hoodi bindings differ from saved manifest"
    );
    let bytecode_hashes = manifest
        .get("bytecodeHashes")
        .context("manifest is missing bytecode hashes")?;
    for (name, address) in [
        ("client", ethereum.client_address),
        ("verifier", ethereum.verifier_address),
        ("queue", ethereum.queue_address),
        ("receiver", ethereum.receiver_address),
    ] {
        let code = provider
            .get_code_at(address)
            .await
            .with_context(|| format!("read Hoodi {name} bytecode"))?;
        ensure!(!code.is_empty(), "Hoodi {name} has no deployed bytecode");
        let actual = B256::from(keccak256(code.as_ref()));
        let expected = manifest_b256(bytecode_hashes, name)?;
        ensure!(
            actual == expected,
            "Hoodi {name} bytecode differs from manifest"
        );
    }
    let current = ethereum.checkpoint().await?;
    ensure!(
        current.block > mmr_start_block
            && current.source_timestamp_ms != 0
            && current.current_len > 0
            && current.current_len <= 256
            && current.next_len > 0
            && current.next_len <= 256
            && current.current_id.checked_add(1) == Some(current.next_id),
        "current Hoodi bootstrap state is invalid"
    );
    Ok(())
}

fn address_string(address: Address) -> String {
    format!("0x{}", hex::encode(address.as_slice()))
}

fn parse_address(value: &Value) -> Option<Address> {
    if let Some(value) = value.as_str() {
        return Address::from_str(value).ok();
    }
    value.get("value").and_then(parse_address)
}

#[cfg(test)]
fn return_address(returns: &Value, name: &str) -> Option<Address> {
    returns.as_object()?.get(name).and_then(parse_address)
}

#[cfg(test)]
fn deployment_transactions(deployment: &Value) -> Result<Vec<Value>> {
    let receipts = deployment
        .get("receipts")
        .and_then(Value::as_array)
        .context("Foundry deployment receipts are missing")?;
    ensure!(!receipts.is_empty(), "Foundry deployment has no receipts");
    receipts
        .iter()
        .enumerate()
        .map(|(index, receipt)| {
            let transaction_hash = receipt
                .get("transactionHash")
                .and_then(Value::as_str)
                .context("Foundry deployment receipt has no transaction hash")?;
            let status = receipt
                .get("status")
                .cloned()
                .context("Foundry deployment receipt has no status")?;
            let gas_used = receipt
                .get("gasUsed")
                .cloned()
                .context("Foundry deployment receipt has no gas used")?;
            ensure!(
                !status.is_null() && !gas_used.is_null(),
                "Foundry deployment receipt has null status or gas"
            );
            Ok(json!({
                "label": format!("deployment-{index}"),
                "txHash": transaction_hash,
                "status": status,
                "gasUsed": gas_used,
                "events": receipt.get("logs").cloned().unwrap_or_else(|| json!([])),
                "checkedEvents": []
            }))
        })
        .collect()
}

pub(crate) fn receipt_value(
    label: &str,
    receipt: &alloy::rpc::types::TransactionReceipt,
    account_nonce: Option<u64>,
) -> Value {
    let events: Vec<Value> = receipt
        .as_ref()
        .logs()
        .iter()
        .map(|log| {
            json!({
                "address": address_string(log.address()),
                "topics": log.topics().iter().map(|topic| format!("0x{}", hex::encode(topic.as_slice()))).collect::<Vec<_>>(),
                "data": format!("0x{}", hex::encode(log.data().data.as_ref()))
            })
        })
        .collect();
    let mut value = json!({
        "label": label,
        "txHash": format!("0x{}", hex::encode(receipt.transaction_hash.as_slice())),
        "status": receipt.status(),
        "gasUsed": receipt.gas_used,
        "blockNumber": receipt.block_number,
        "blockHash": receipt.block_hash.map(|hash| format!("{hash:#x}")),
        "events": events,
        "checkedEvents": []
    });
    if let Some(account_nonce) = account_nonce {
        value["accountNonce"] = json!(account_nonce);
    }
    value
}

fn commitment_payload(payload: &impl Encode) -> Result<Vec<BeefyClient::PayloadItem>> {
    let bytes = payload.encode();
    let mut input = &bytes[..];
    let entries: Vec<([u8; 2], Vec<u8>)> =
        Decode::decode(&mut input).context("decode BEEFY payload entries")?;
    ensure!(input.is_empty(), "BEEFY payload has trailing bytes");
    Ok(entries
        .into_iter()
        .map(|(payload_id, data)| BeefyClient::PayloadItem {
            payloadID: FixedBytes::from(payload_id),
            data: Bytes::from(data),
        })
        .collect())
}
fn to_sol_snapshot(snapshot: &QueueSnapshot) -> VaraBridgeMetadata::Snapshot {
    VaraBridgeMetadata::Snapshot {
        version: SNAPSHOT_VERSION,
        bridgeDomain: B256::from(snapshot.bridge_domain),
        sourceTimestampMs: snapshot.source_timestamp_ms,
        initialized: snapshot.initialized,
        queueId: snapshot.queue_id,
        queueRoot: B256::from(snapshot.queue_root),
    }
}

fn to_sol_leaf(leaf: &RuntimeLeaf) -> Result<BeefyClient::MMRLeaf> {
    let version = leaf.version.encode();
    ensure!(
        version.len() == 1 && leaf.version.split() == (0, 0),
        "unsupported MMR leaf version"
    );
    Ok(BeefyClient::MMRLeaf {
        version: version[0],
        parentNumber: leaf.parent_number_and_hash.0,
        parentHash: B256::from(leaf.parent_number_and_hash.1),
        nextAuthoritySetID: leaf.beefy_next_authority_set.id,
        nextAuthoritySetLen: leaf.beefy_next_authority_set.len,
        nextAuthoritySetRoot: B256::from(leaf.beefy_next_authority_set.keyset_commitment),
        parachainHeadsRoot: B256::from(leaf.leaf_extra),
    })
}
#[cfg(test)]
fn redact_process_output(bytes: &[u8], private_key: &str) -> String {
    let mut output = String::from_utf8_lossy(bytes).into_owned();
    output = output.replace(private_key, "<redacted>");
    if let Some(raw) = private_key
        .strip_prefix("0x")
        .or_else(|| private_key.strip_prefix("0X"))
    {
        output = output.replace(raw, "<redacted>");
    }
    output
}
#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{SignableTransaction, TxEip1559, TxLegacy},
        network::TxSigner,
        primitives::TxKind,
    };

    #[tokio::test]
    async fn bootstrap_deployment_hint_authenticates_creation_and_restart() -> Result<()> {
        use alloy::{
            network::TransactionBuilder, rpc::types::TransactionRequest, sol_types::SolValue,
        };
        let anvil = Anvil::new()
            .host("127.0.0.1")
            .port(0u16)
            .chain_id(31_337)
            .arg("--slots-in-an-epoch")
            .arg("1")
            .arg("--timestamp")
            .arg("1700000000")
            .try_spawn()?;
        let key = format!("0x{}", hex::encode(anvil.first_key().to_bytes()));
        let queue = Address::from([3; 20]);
        let api = EthApi::new(
            &anvil.ws_endpoint(),
            &address_string(queue),
            Some(&key),
            None,
            None,
        )
        .await?;
        let provider = api.raw_provider().clone();
        let arguments = (
            B256::from([1; 32]),
            U256::from(31_337),
            queue,
            1u64,
            2u64,
            1_700_000_000_000u64,
            (7u128, 1u128, B256::from([4; 32])),
            (8u128, 1u128, B256::from([5; 32])),
        )
            .abi_encode();
        let input = [BeefyClient::BYTECODE.as_ref(), arguments.as_slice()].concat();
        let receipt = provider
            .send_transaction(
                TransactionRequest::default()
                    .with_deploy_code(input)
                    .with_gas_limit(6_000_000),
            )
            .await?
            .get_receipt()
            .await?;
        assert!(receipt.status());
        let wrong = provider
            .send_transaction(
                TransactionRequest::default()
                    .with_to(queue)
                    .with_gas_limit(21_000),
            )
            .await?
            .get_receipt()
            .await?;
        provider
            .client()
            .request::<_, Value>("anvil_mine", ("0x3",))
            .await?;
        let ethereum = Ethereum {
            _anvil: Some(anvil),
            owned_fixture_network: None,
            api,
            public_network: PublicEthereumNetwork::Hoodi,
            client_address: receipt.contract_address.context("missing created client")?,
            verifier_address: Address::ZERO,
            queue_address: queue,
            receiver_address: Address::ZERO,
            transactions: Vec::new(),
        };
        let directory = tempfile::tempdir()?;
        let (checkpoint, pin) = ethereum
            .authenticate_bootstrap(
                directory.path(),
                &Value::Null,
                Some(receipt.transaction_hash),
            )
            .await?;
        assert_eq!(checkpoint.block, 2);
        assert_eq!(checkpoint.source_timestamp_ms, 1_700_000_000_000);
        assert_eq!(
            pin,
            json!({"block":receipt.block_number, "blockHash":receipt.block_hash,
            "txHash":receipt.transaction_hash})
        );
        for hint in [None, Some(receipt.transaction_hash)] {
            let (restored, restored_pin) = ethereum
                .authenticate_bootstrap(directory.path(), &pin, hint)
                .await?;
            assert_eq!(restored, checkpoint);
            assert_eq!(restored_pin, pin);
        }
        let missing = ethereum
            .authenticate_bootstrap(directory.path(), &Value::Null, Some(B256::ZERO))
            .await
            .unwrap_err();
        assert!(
            missing.to_string().contains("receipt unavailable"),
            "{missing:#}"
        );
        let unrelated = ethereum
            .authenticate_bootstrap(directory.path(), &Value::Null, Some(wrong.transaction_hash))
            .await
            .unwrap_err();
        assert!(
            unrelated.to_string().contains("signed identity"),
            "{unrelated:#}"
        );
        let conflicting = ethereum
            .authenticate_bootstrap(directory.path(), &pin, Some(wrong.transaction_hash))
            .await
            .unwrap_err();
        assert!(
            conflicting
                .to_string()
                .contains("conflicts with saved bootstrap"),
            "{conflicting:#}"
        );
        let mut changed = pin.clone();
        changed["blockHash"] = json!(B256::from([9; 32]));
        assert!(ethereum
            .authenticate_bootstrap(directory.path(), &changed, Some(receipt.transaction_hash))
            .await
            .is_err());
        changed = pin.clone();
        changed["block"] = json!(receipt.block_number.unwrap() + 1);
        assert!(ethereum
            .authenticate_bootstrap(directory.path(), &changed, None)
            .await
            .is_err());
        // Archive discovery remains available without a locator and cannot change the pin.
        let (discovered, discovered_pin) = ethereum
            .authenticate_bootstrap(directory.path(), &Value::Null, None)
            .await?;
        assert_eq!(discovered, checkpoint);
        assert_eq!(discovered_pin, pin);
        assert!(
            ethereum.transactions.is_empty(),
            "bootstrap reads must not sign or populate submissions"
        );
        Ok(())
    }

    #[tokio::test]
    #[ignore = "read-only original deployment and independently compiled bytecode comparison"]
    async fn actual_client_creation_matches_compiled_approved_bytecode() -> Result<()> {
        let evidence: Value = serde_json::from_slice(&fs::read(std::env::var(
            "BEEFY_TEST_CLIENT_DEPLOYMENT_EVIDENCE",
        )?)?)?;
        let original = evidence["originals"]
            .as_array()
            .context("missing originals")?
            .iter()
            .find(|entry| entry["contractName"] == "BeefyClient")
            .context("missing client creation")?;
        let hash = manifest_b256(original, "hash")?;
        let raw = hex::decode(
            original["raw"]
                .as_str()
                .context("missing signed original")?
                .trim_start_matches("0x"),
        )?;
        ensure!(
            B256::from(keccak256(&raw)) == hash,
            "original raw hash changed"
        );
        let mut encoded = raw.as_slice();
        let signed = TxEnvelope::decode_2718(&mut encoded)?;
        ensure!(
            encoded.is_empty()
                && signed.to().is_none()
                && signed.recover_signer()?.create(signed.nonce())
                    == manifest_address(original, "contractAddress")?,
            "original CREATE identity changed"
        );
        let endpoint = std::env::var("BEEFY_TEST_HOODI_RPC")?;
        let api = PollingEthApi::new(&endpoint).await?;
        let observed = api
            .get_transaction_by_hash(hash)
            .await?
            .context("creation unavailable")?
            .into_inner();
        ensure!(
            observed.encoded_2718() == raw,
            "live creation differs from saved signed original"
        );
        let code = BeefyClient::BYTECODE.as_ref();
        ensure!(
            observed.input().starts_with(code),
            "actual creation does not match compiled approved BYTECODE"
        );
        let artifact: Value =
            serde_json::from_slice(&fs::read(std::env::var("BEEFY_TEST_CLIENT_ARTIFACT")?)?)?;
        let artifact_code = hex::decode(
            artifact["bytecode"]["object"]
                .as_str()
                .context("missing artifact bytecode")?
                .trim_start_matches("0x"),
        )?;
        ensure!(
            artifact_code == code,
            "coherent deployment artifact differs from compiled approval"
        );
        println!(
            "creation={hash:#x} approved_bytes={} constructor_bytes={} approved_keccak={:#x}",
            code.len(),
            observed.input().len() - code.len(),
            B256::from(keccak256(code))
        );
        Ok(())
    }

    #[test]
    fn owned_fixture_descriptor_refuses_missing_approval_and_rebinding() -> Result<()> {
        let network = OwnedFixtureNetwork {
            schema_version: 1,
            ethereum_chain_id: 31_337,
            ethereum_genesis_hash: B256::from([1; 32]),
            beacon_genesis_validators_root: B256::from([2; 32]),
            beacon_genesis_time: 1_700_000_000,
        };
        let encoded = serde_json::to_value(&network)?;
        assert_eq!(
            serde_json::from_value::<OwnedFixtureNetwork>(encoded.clone())?,
            network
        );
        let mut unknown = encoded.clone();
        unknown["trustLiveRpc"] = json!(true);
        assert!(serde_json::from_value::<OwnedFixtureNetwork>(unknown).is_err());
        let mut malformed = encoded.clone();
        malformed["ethereumGenesisHash"] = json!("0x01");
        assert!(serde_json::from_value::<OwnedFixtureNetwork>(malformed).is_err());
        for field in [
            "schemaVersion",
            "ethereumChainId",
            "ethereumGenesisHash",
            "beaconGenesisValidatorsRoot",
            "beaconGenesisTime",
        ] {
            let mut missing = encoded.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<OwnedFixtureNetwork>(missing).is_err());
        }
        let deployment = json!({"localRehearsal":false, "ethereum":{"chainId":31337}, "ownedFixtureNetwork":encoded});
        let endpoints = (
            "ws://127.0.0.1:9944",
            "ws://127.0.0.1:9945",
            "ws://127.0.0.1:8546",
        );
        validate_fixture_approval(
            Some(&network),
            &deployment,
            endpoints.0,
            endpoints.1,
            endpoints.2,
            PublicEthereumNetwork::Hoodi,
        )?;
        assert!(validate_fixture_approval(
            None,
            &deployment,
            endpoints.0,
            endpoints.1,
            endpoints.2,
            PublicEthereumNetwork::Hoodi
        )
        .is_err());
        assert!(validate_fixture_approval(
            Some(&network),
            &deployment,
            endpoints.0,
            endpoints.1,
            endpoints.2,
            PublicEthereumNetwork::Mainnet
        )
        .is_err());
        for endpoint in [
            "ws://localhost:8546",
            "wss://ethereum-hoodi-rpc.publicnode.com",
            "ws://192.168.1.1:8546",
            "ws://127.0.0.1:8546/path",
            "ws://user@127.0.0.1:8546",
        ] {
            assert!(network
                .validate_endpoints(endpoints.0, endpoints.1, endpoint)
                .is_err());
        }
        for field in [
            "ethereumGenesisHash",
            "beaconGenesisValidatorsRoot",
            "beaconGenesisTime",
        ] {
            let mut changed = deployment.clone();
            changed["ownedFixtureNetwork"][field] = if field == "beaconGenesisTime" {
                json!(1_700_000_001)
            } else {
                json!(B256::from([3; 32]))
            };
            assert!(network.validate_manifest(&changed).is_err());
        }
        let mut rehearsal = deployment.clone();
        rehearsal["localRehearsal"] = json!(true);
        assert!(network.validate_manifest(&rehearsal).is_err());
        for field in [
            "schemaVersion",
            "ethereumChainId",
            "ethereumGenesisHash",
            "beaconGenesisValidatorsRoot",
            "beaconGenesisTime",
        ] {
            let mut invalid = serde_json::to_value(&network)?;
            invalid[field] = if field.ends_with("Hash") || field.ends_with("Root") {
                json!(B256::ZERO)
            } else {
                json!(0)
            };
            assert!(serde_json::from_value::<OwnedFixtureNetwork>(invalid)?
                .validate()
                .is_err());
        }
        let hoodi = json!({"ethereum":{"chainId":HOODI_CHAIN_ID}});
        validate_fixture_approval(
            None,
            &hoodi,
            endpoints.0,
            endpoints.1,
            endpoints.2,
            PublicEthereumNetwork::Hoodi,
        )?;
        assert!(network.validate_manifest(&hoodi).is_err());
        Ok(())
    }

    #[test]
    fn mainnet_admission_requires_explicit_matching_public_identity() -> Result<()> {
        use PublicEthereumNetwork::{Hoodi, Mainnet};
        let mainnet =
            json!({"mode":"mainnet-token-stack", "localRehearsal":false, "ethereum":{"chainId":1}});
        let check = |deployment: &Value, network, endpoint| {
            validate_fixture_approval(
                None,
                deployment,
                "wss://source.example",
                "wss://witness.example",
                endpoint,
                network,
            )
        };
        check(&mainnet, Mainnet, "https://ethereum.example")?;
        assert!(check(&mainnet, Hoodi, "https://ethereum.example").is_err());
        for endpoint in ["http://ethereum.example", "ws://127.0.0.1:8546", ""] {
            assert!(check(&mainnet, Mainnet, endpoint).is_err());
        }
        for (pointer, value) in [
            ("/mode", json!("hoodi-token-stack")),
            ("/localRehearsal", json!(true)),
            ("/ethereum/chainId", json!(560048)),
            ("/ethereum/chainId", json!(31337)),
        ] {
            let mut changed = mainnet.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(check(&changed, Mainnet, "https://ethereum.example").is_err());
        }
        let mut fixture = mainnet;
        fixture["ownedFixtureNetwork"] = json!({});
        assert!(check(&fixture, Mainnet, "https://ethereum.example").is_err());
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires an explicit read-only Ethereum mainnet endpoint; no signer or deployment"]
    async fn qualify_mainnet_execution_finality_read_only() -> Result<()> {
        let endpoint = std::env::var("BEEFY_TEST_MAINNET_RPC")?;
        let network = PublicEthereumNetwork::Mainnet;
        ensure_ethereum_network(&endpoint, network.chain_id(), network.genesis()).await?;
        ensure!(
            ensure_ethereum_network(&endpoint, HOODI_CHAIN_ID, HOODI_GENESIS_BLOCK)
                .await
                .is_err(),
            "mainnet endpoint was admitted as Hoodi"
        );
        let api = EthApi::new(&endpoint, &address_string(Address::ZERO), None, None, None).await?;
        let directory = tempfile::tempdir()?;
        let view = api
            .verified_finalized_view_with_archive(directory.path(), network.genesis())
            .await?;
        let header = api
            .raw_provider()
            .get_block_by_hash(view.block_hash())
            .await?
            .context("finalized mainnet header missing")?;
        ensure!(
            header.header.number == view.block_number()
                && header.header.inner.hash_slow() == view.block_hash(),
            "mainnet finalized identity changed"
        );
        println!(
            "chain={} genesis={:#x} finalized={} hash={:#x}",
            network.chain_id(),
            network.genesis(),
            view.block_number(),
            view.block_hash()
        );
        Ok(())
    }

    #[tokio::test]
    async fn owned_fixture_and_default_hoodi_require_exact_network_identity() -> Result<()> {
        use alloy::{providers::ProviderBuilder, transports::mock::Asserter};
        let mut block: alloy::rpc::types::Block = Default::default();
        block.header.hash = block.header.inner.hash_slow();
        for expected_chain in [1, HOODI_CHAIN_ID, 31_337] {
            for (observed_chain, expected_genesis, accepted) in [
                (expected_chain, block.header.hash, true),
                (expected_chain + 1, block.header.hash, false),
                (expected_chain, B256::from([9; 32]), false),
            ] {
                let asserter = Asserter::new();
                let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
                asserter.push_success(&format!("0x{observed_chain:x}"));
                asserter.push_success(&block);
                assert_eq!(
                    verify_ethereum_network(&provider, expected_chain, expected_genesis)
                        .await
                        .is_ok(),
                    accepted
                );
            }
        }
        assert!(Ethereum::connect_readonly(
            "ws://127.0.0.1:1",
            &json!({"chainId":31337}),
            None,
            PublicEthereumNetwork::Hoodi
        )
        .await
        .is_err());
        Ok(())
    }

    #[tokio::test]
    async fn missing_receipt_header_waits_without_accepting_a_changed_inclusion() -> Result<()> {
        use alloy::{providers::ProviderBuilder, transports::mock::Asserter};
        let included_hash = B256::from([1; 32]);
        for canonical_hash in [included_hash, B256::from([2; 32])] {
            let asserter = Asserter::new();
            let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
            let mut block: alloy::rpc::types::Block = Default::default();
            block.header.hash = canonical_hash;
            block.header.inner.number = 42;
            asserter.push_success(&Option::<Value>::None);
            asserter.push_success(&block);
            let result = wait_canonical_receipt(&provider, 42, included_hash).await;
            if canonical_hash == included_hash {
                result?;
            } else {
                assert!(result.is_err(), "a changed inclusion must not be accepted");
            }
        }
        Ok(())
    }
    #[tokio::test]
    async fn retained_transaction_and_receipt_nodes_reject_changed_roots_indices_and_payloads(
    ) -> Result<()> {
        use alloy::consensus::{
            proofs::{calculate_receipt_root, calculate_transaction_root},
            Receipt, ReceiptEnvelope,
        };
        let signer = PrivateKeySigner::from_bytes(&B256::from([17; 32]))?;
        let mut tx = TxEip1559 {
            chain_id: HOODI_CHAIN_ID,
            nonce: 7,
            gas_limit: 21_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(Address::from([18; 20])),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut tx).await?;
        let transaction = TxEnvelope::Eip1559(tx.into_signed(signature));
        let raw = transaction.encoded_2718();
        let root = calculate_transaction_root(std::slice::from_ref(&transaction));
        let nodes = transaction_proof(&[transaction], 0, root)?;
        verify_indexed_value(root, 0, &raw, &nodes)?;
        for (changed_root, changed_index, changed_value, changed_nodes) in [
            (B256::ZERO, 0, raw.clone(), nodes.clone()),
            (root, 1, raw.clone(), nodes.clone()),
            (root, 0, vec![0xff], nodes.clone()),
            (root, 0, raw, Vec::new()),
        ] {
            assert!(verify_indexed_value(
                changed_root,
                changed_index,
                &changed_value,
                &changed_nodes
            )
            .is_err());
        }
        let receipt = ReceiptEnvelope::Eip1559(
            Receipt::<alloy::primitives::Log> {
                status: true.into(),
                cumulative_gas_used: 21_000,
                logs: Vec::new(),
            }
            .with_bloom(),
        );
        let root = calculate_receipt_root(std::slice::from_ref(&receipt));
        let proof = ethereum_common::utils::generate_merkle_proof(0, &[(0, receipt.clone())])?;
        verify_indexed_value(root, 0, &receipt.encoded_2718(), &proof.proof)?;
        assert!(verify_indexed_value(root, 1, &receipt.encoded_2718(), &proof.proof).is_err());
        assert!(verify_indexed_value(root, 0, &[0xff], &proof.proof).is_err());
        assert!(verify_indexed_value(root, 0, &receipt.encoded_2718(), &[]).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn signed_identity_rejects_replacement_nonce_destination_sender_and_trailing_bytes(
    ) -> Result<()> {
        let signer = PrivateKeySigner::from_bytes(&B256::from([7; 32]))?;
        let destination = Address::from([3; 20]);
        let mut tx = TxEip1559 {
            chain_id: HOODI_CHAIN_ID,
            nonce: 36,
            gas_limit: 21_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(destination),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut tx).await?;
        let raw = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        let hash = B256::from(keccak256(&raw));
        let signed = signed_transaction_identity(&raw, hash, 36, destination)?;
        assert_eq!(signed.from, signer.address());
        assert!(signed_transaction_identity(&raw, B256::ZERO, 36, destination).is_err());
        assert!(signed_transaction_identity(&raw, hash, 37, destination).is_err());
        assert!(signed_transaction_identity(&raw, hash, 36, Address::from([4; 20])).is_err());
        let mut trailing = raw.clone();
        trailing.push(0);
        assert!(signed_transaction_identity(
            &trailing,
            B256::from(keccak256(&trailing)),
            36,
            destination
        )
        .is_err());
        assert!(signed_transaction_identity(
            &[0xff],
            B256::from(keccak256(&[0xff])),
            36,
            destination
        )
        .is_err());
        for changed in [
            TransactionIdentity {
                from: Address::from([5; 20]),
                ..signed
            },
            TransactionIdentity {
                nonce: 37,
                ..signed
            },
            TransactionIdentity {
                to: Some(Address::from([6; 20])),
                ..signed
            },
            TransactionIdentity {
                hash: B256::ZERO,
                ..signed
            },
        ] {
            assert!(ensure_same_submission(changed, signed).is_err());
        }
        let mut legacy = TxLegacy {
            chain_id: Some(HOODI_CHAIN_ID),
            nonce: 0,
            gas_price: 10,
            gas_limit: 21_000,
            to: TxKind::Call(destination),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut legacy).await?;
        let legacy = TxEnvelope::Legacy(legacy.into_signed(signature)).encoded_2718();
        assert_eq!(
            signed_transaction_identity(&legacy, B256::from(keccak256(&legacy)), 0, destination)?
                .from,
            signer.address()
        );
        Ok(())
    }

    #[tokio::test]
    async fn disappeared_original_rebroadcasts_same_hash_or_holds_consumed_nonce() -> Result<()> {
        use alloy::{providers::ProviderBuilder, transports::mock::Asserter};
        let signer = PrivateKeySigner::from_bytes(&B256::from([9; 32]))?;
        let destination = Address::from([8; 20]);
        let mut tx = TxEip1559 {
            chain_id: HOODI_CHAIN_ID,
            nonce: 36,
            gas_limit: 21_000,
            max_fee_per_gas: 10,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(destination),
            ..Default::default()
        };
        let signature = signer.sign_transaction(&mut tx).await?;
        let raw = TxEnvelope::Eip1559(tx.into_signed(signature)).encoded_2718();
        let hash = B256::from(keccak256(&raw));
        let signed = signed_transaction_identity(&raw, hash, 36, destination)?;
        let asserter = Asserter::new();
        let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
        let seen = json!({"hash": format!("{hash:#x}"), "from": format!("{:#x}", signed.from),
            "nonce": "0x24", "to": format!("{destination:#x}"), "blockNumber": null});
        asserter.push_success(&Option::<Value>::None); // receipt absent
        asserter.push_success(&seen); // original still pending
        asserter.push_success(&Option::<Value>::None); // then dropped
        asserter.push_success(&Option::<Value>::None);
        asserter.push_success(&"0x24"); // latest
        asserter.push_success(&"0x24"); // pending
        asserter.push_success(&hash); // exact original raw broadcast
        assert!(
            timeout(
                Duration::from_secs(3),
                await_original_commitment(&provider, signed, &raw)
            )
            .await
            .is_err(),
            "mined receipt must not be invented after rebroadcast"
        );
        assert!(
            asserter.read_q().is_empty(),
            "original broadcast did not consume its response"
        );

        asserter.push_success(&Option::<Value>::None);
        asserter.push_success(&Option::<Value>::None);
        asserter.push_success(&"0x25"); // another transaction consumed the reserved nonce
        asserter.push_success(&"0x25");
        let hold = await_original_commitment(&provider, signed, &raw)
            .await
            .unwrap_err();
        assert!(
            hold.to_string().contains("HOLD: original commitment"),
            "{hold:#}"
        );
        assert!(
            asserter.read_q().is_empty(),
            "no replacement submission may be attempted"
        );
        Ok(())
    }
    #[tokio::test]
    #[ignore = "read-only real Hoodi RPC smoke"]
    async fn original_hoodi_hash_has_canonical_signed_identity() -> Result<()> {
        let endpoint = std::env::var("BEEFY_TEST_HOODI_RPC")
            .unwrap_or_else(|_| "https://rpc.sentio.xyz/hoodi".to_owned());
        let api = PollingEthApi::new(&endpoint).await?;
        let hash: B256 =
            "0x3ab31061459478dd7625369532a2cf5e4c79204d7345ccca094290be8c7c1946".parse()?;
        let identity = transaction_identity(&*api, hash)
            .await?
            .context("original Hoodi transaction is missing")?;
        ensure!(
            identity.hash == hash
                && identity.from
                    == "0x312684c68309d41964d6229959e4671bb8d3878c".parse::<Address>()?
                && identity.nonce == 36
                && identity.to == Some("0xc040a1bf14a398df2d0bd02a24fc18e5244b0e66".parse()?),
            "original Hoodi transaction identity changed"
        );
        let finalized = api
            .get_finalized_receipt(hash)
            .await?
            .context("original Hoodi receipt is not finalized")?;
        ensure!(
            finalized.receipt.transaction_hash == hash
                && finalized.receipt.from == identity.from
                && finalized.receipt.to == identity.to
                && finalized.receipt.status()
                && identity.block_number == Some(finalized.included_block_number)
                && finalized.receipt.block_hash == Some(finalized.included_block_hash),
            "original Hoodi inclusion differs from finalized receipt"
        );
        Ok(())
    }
}
