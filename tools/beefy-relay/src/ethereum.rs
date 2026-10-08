use alloy::{
    consensus::{transaction::SignerRecoverable, Transaction as _, TxEnvelope},
    eips::{Decodable2718, Encodable2718},
    node_bindings::{Anvil, AnvilInstance},
    primitives::{Address, Bytes, FixedBytes, B256, U256},
    providers::{Provider, WalletProvider},
    rpc::types::{BlockId, BlockNumberOrTag},
    signers::local::PrivateKeySigner,
    sol,
};
use anyhow::{bail, ensure, Context, Result};
use ethereum_client::{
    abi::IMessageQueue::IMessageQueueErrors, transaction_identity, Error as EthereumClientError,
    EthApi, PollingEthApi, TransactionIdentity,
};
use gear_rpc_client::dto::{MerkleProof, Message};
use parity_scale_codec::{Decode, Encode};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    str::FromStr,
    time::Duration,
};
use tokio::{
    process::Command,
    time::{sleep, timeout, Instant},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use crate::source::CapturedCommitment;
use beefy_relay::{
    authority_proof, keccak256, Hash32, QueueSnapshot, RuntimeLeaf, SimplifiedMmrProof,
    SNAPSHOT_VERSION,
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
                        && receipt.to == signed.to
                        && receipt.status(),
                    "commitment receipt identity or status differs from signed intent"
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
    let original = beefy_relay::decode_versioned_finality_proof(source_raw)?;
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
    header: alloy::consensus::Header,
    index: u64,
    raw_transaction: Vec<u8>,
    receipt: alloy::consensus::ReceiptEnvelope,
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
pub(crate) const HOODI_GENESIS_BLOCK: &str =
    "0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b";
const HOODI_DEFAULT_RPC: &str = "wss://ethereum-hoodi-rpc.publicnode.com";
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
        function recoveryController() external view returns (address);
        function maxBlockNumber() external view returns (uint256);
    }
    #[sol(rpc)]
    interface TokenRecoveryControllerBinding {
        function messageQueue() external view returns (address);
        function recoveryWallet() external view returns (address);
        function RECOVERY_DELAY() external view returns (uint256);
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
    pub api: EthApi,
    pub client_address: Address,
    pub verifier_address: Address,
    pub queue_address: Address,
    pub receiver_address: Address,
    pub transactions: Vec<Value>,
}

impl Ethereum {
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
        // Only this method owns a disposable 31337 Anvil; public deployments require a real wallet.
        let test_key = format!(
            "0x{}",
            hex::encode(
                anvil
                    .keys()
                    .get(1)
                    .context("owned Anvil has no second test account")?
                    .to_bytes()
            )
        );
        let test_wallet = Command::new("forge")
            .args([
                "create",
                "test/RecoverySafeMock.sol:RecoverySafeTestWallet",
                "--root",
                "ethereum",
                "--rpc-url",
                &endpoint,
                "--private-key",
                &test_key,
                "--broadcast",
                "--json",
                "--constructor-args",
                "3",
                "5",
            ])
            .current_dir(&root)
            .env("FOUNDRY_BROADCAST", &broadcast_dir)
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output()
            .await?;
        ensure!(
            test_wallet.status.success(),
            "owned-Anvil test wallet deployment failed: {} {}",
            redact_process_output(&test_wallet.stdout, &test_key),
            redact_process_output(&test_wallet.stderr, &test_key)
        );
        let test_wallet: Value = serde_json::from_slice(&test_wallet.stdout)?;
        let recovery_wallet = manifest_address(&test_wallet, "deployedTo")?;
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
            .env("BEEFY_RECOVERY_WALLET", address_string(recovery_wallet))
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

    pub async fn prepare_hoodi(
        output: &Path,
        checkpoint: &CapturedCommitment,
        snapshot: &QueueSnapshot,
        mmr_start_block: u64,
        endpoint: &str,
        wallet: &Path,
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

        let (_wallet_address, private_key) = read_hoodi_wallet(wallet)?;
        let endpoint = hoodi_ws_endpoint(endpoint)?;
        ensure_hoodi_network(&endpoint).await?;

        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let current = &checkpoint.current;
        let next = &checkpoint.next;
        let source_domain = B256::from_str(
            &std::env::var("BEEFY_SOURCE_DOMAIN").context("BEEFY_SOURCE_DOMAIN required")?,
        )?
        .0;
        let forge = Command::new("forge")
            .args([
                "script",
                "ethereum/script/BeefyHoodi.s.sol:BeefyHoodi",
                "--force",
                "--root",
                "ethereum",
                "--rpc-url",
                &endpoint,
                "--broadcast",
                "--slow",
                "--sig",
                "run()",
            ])
            .current_dir(&root)
            .env("PRIVATE_KEY", &private_key)
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
            .context("run BeefyHoodi Foundry deployment")?;
        ensure!(
            forge.status.success(),
            "BeefyHoodi deployment failed with status {} (forge output withheld)",
            forge.status
        );

        let broadcast = root.join("ethereum/broadcast/BeefyHoodi.s.sol/560048/run-latest.json");
        ensure!(
            broadcast.is_file(),
            "Foundry Hoodi deployment broadcast is missing"
        );
        let broadcast_bytes =
            fs::read(&broadcast).context("read Foundry Hoodi deployment broadcast")?;
        if output.is_dir() {
            fs::write(output.join("ethereum-broadcast.json"), &broadcast_bytes)
                .context("copy Foundry Hoodi deployment broadcast")?;
        }
        let deployment: Value = serde_json::from_slice(&broadcast_bytes)
            .context("decode Foundry Hoodi deployment broadcast")?;
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
            &endpoint,
            &address_string(queue_address),
            Some(&private_key),
            None,
            None,
        )
        .await
        .context("connect Ethereum client to Hoodi")?;
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
                .context("read deployed Hoodi bytecode")?;
            ensure!(
                !code.is_empty(),
                "Hoodi deployment returned an address without code"
            );
        }

        let ethereum = Self {
            _anvil: None,
            api,
            client_address,
            verifier_address,
            queue_address,
            receiver_address,
            transactions,
        };
        verify_hoodi_bootstrap(&ethereum, checkpoint, snapshot, mmr_start_block).await?;
        Ok(ethereum)
    }

    pub async fn connect_hoodi(endpoint: &str, wallet: &Path, manifest: &Value) -> Result<Self> {
        let ethereum = Self::connect_hoodi_addresses(endpoint, wallet, manifest).await?;
        verify_hoodi_manifest(&ethereum, manifest).await?;
        Ok(ethereum)
    }
    /// Read the live deployment before pinning its immutable addresses and bytecode hashes.
    pub async fn token_manifest(endpoint: &str, wallet: &Path, addresses: &Value) -> Result<Value> {
        let ethereum = Self::connect_hoodi_addresses(endpoint, wallet, addresses).await?;
        verify_secure_policy(ethereum.client_address, ethereum.api.raw_provider().clone()).await?;
        verify_bindings(
            ethereum.verifier_address,
            ethereum.client_address,
            ethereum.queue_address,
            HOODI_CHAIN_ID,
            ethereum.api.raw_provider().clone(),
        )
        .await?;
        let manifest = ethereum.manifest().await?;
        verify_hoodi_manifest(&ethereum, &manifest).await?;
        Ok(manifest)
    }
    async fn connect_hoodi_addresses(
        endpoint: &str,
        wallet: &Path,
        addresses: &Value,
    ) -> Result<Self> {
        ensure!(
            manifest_u64(addresses, "chainId")? == HOODI_CHAIN_ID,
            "Hoodi manifest chain id is not 560048"
        );
        let client_address = manifest_address(addresses, "client")?;
        let verifier_address = manifest_address(addresses, "verifier")?;
        let queue_address = manifest_address(addresses, "queue")?;
        let receiver_address = manifest_address(addresses, "receiver")?;
        let endpoint = hoodi_ws_endpoint(endpoint)?;
        ensure_hoodi_network(&endpoint).await?;
        let (_wallet_address, private_key) = read_hoodi_wallet(wallet)?;
        let api = EthApi::new(
            &endpoint,
            &address_string(queue_address),
            Some(&private_key),
            None,
            None,
        )
        .await
        .context("connect Ethereum client to Hoodi")?;
        Ok(Self {
            _anvil: None,
            api,
            client_address,
            verifier_address,
            queue_address,
            receiver_address,
            transactions: Vec::new(),
        })
    }
    pub(crate) async fn connect_hoodi_readonly(endpoint: &str, manifest: &Value) -> Result<Self> {
        ensure!(
            manifest_u64(manifest, "chainId")? == HOODI_CHAIN_ID,
            "not a Hoodi deployment"
        );
        let client_address = manifest_address(manifest, "client")?;
        let verifier_address = manifest_address(manifest, "verifier")?;
        let queue_address = manifest_address(manifest, "queue")?;
        let receiver_address = manifest_address(manifest, "receiver")?;
        let endpoint = hoodi_ws_endpoint(endpoint)?;
        ensure_hoodi_network(&endpoint).await?;
        let api = EthApi::new(&endpoint, &address_string(queue_address), None, None, None)
            .await
            .context("connect read-only recovery verifier watcher")?;
        ensure!(
            api.raw_provider().get_chain_id().await? == HOODI_CHAIN_ID,
            "recovery watcher is not connected to Hoodi"
        );
        Ok(Self {
            _anvil: None,
            api,
            client_address,
            verifier_address,
            queue_address,
            receiver_address,
            transactions: Vec::new(),
        })
    }

    pub(crate) async fn finality_genesis(&self) -> Result<B256> {
        if self._anvil.is_none() {
            return Ok(HOODI_GENESIS_BLOCK.parse()?);
        }
        let genesis = self
            .api
            .raw_provider()
            .get_block_by_number(0u64.into())
            .await?
            .context("owned rehearsal genesis is missing")?;
        ensure!(
            genesis.header.number == 0 && genesis.header.inner.hash_slow() == genesis.header.hash,
            "owned rehearsal genesis is not canonical RLP; HOLD"
        );
        Ok(genesis.header.hash)
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

    pub(crate) async fn connect_hoodi_active(
        endpoint: &str,
        wallet: &Path,
        manifest: &Value,
    ) -> Result<Self> {
        let ethereum = Self::connect_hoodi_addresses(endpoint, wallet, manifest).await?;
        verify_hoodi_manifest(&ethereum, manifest).await?;
        Ok(ethereum)
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

    pub fn receiver_address(&self) -> [u8; 20] {
        self.receiver_address.into()
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
        let provider = self.api.raw_provider().clone();
        let recovery_controller = TokenQueueBinding::new(self.queue_address, provider.clone())
            .recoveryController()
            .call()
            .await?;
        ensure!(
            recovery_controller != Address::ZERO,
            "fresh BEEFY queue has no installed recovery controller"
        );
        let recovery = TokenRecoveryControllerBinding::new(recovery_controller, provider.clone());
        let recovery_wallet = recovery.recoveryWallet().call().await?;
        ensure!(
            recovery.messageQueue().call().await? == self.queue_address
                && recovery_wallet != Address::ZERO
                && recovery.RECOVERY_DELAY().call().await? == U256::from(24 * 60 * 60u64),
            "fresh BEEFY recovery controller/wallet binding or 24-hour delay is invalid"
        );
        for address in [recovery_controller, recovery_wallet] {
            ensure!(
                !provider.get_code_at(address).await?.is_empty(),
                "fresh BEEFY recovery controller/wallet has no code"
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
            "recoveryController": address_string(recovery_controller),
            "recoveryWallet": address_string(recovery_wallet),
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

    async fn checkpoint_at(&self, block: BlockId) -> Result<DestinationCheckpoint> {
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
        journal(&transaction, None)?;
        let receipt = self.resume_commitment(&transaction).await?;
        journal(&transaction, Some(&receipt))?;
        ensure!(
            receipt.status(),
            "Fiat-Shamir commitment transaction reverted"
        );
        self.record_receipt("submitFiatShamir", &receipt, None);

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
        ensure!(
            signed.from == provider.default_signer_address(),
            "saved commitment signer differs from follower wallet"
        );
        await_original_commitment(provider, signed, &raw).await
    }

    async fn mined_receipt(&self, hash: B256) -> Result<alloy::rpc::types::TransactionReceipt> {
        loop {
            if let Some(receipt) = self
                .api
                .raw_provider()
                .get_transaction_receipt(hash)
                .await?
            {
                return Ok(receipt);
            }
            sleep(Duration::from_secs(2)).await;
        }
    }

    pub async fn register(&mut self, source: u32, root: Hash32, encoded: Vec<u8>) -> Result<()> {
        let pending = self
            .api
            .provide_merkle_root(source, root, encoded)
            .await
            .context("submit authenticated queue root")?;
        let receipt = self.mined_receipt(*pending.tx_hash()).await?;
        ensure!(receipt.status(), "queue-root transaction reverted");
        self.record_receipt("submitMerkleRoot", &receipt, None);
        ensure!(
            self.api
                .read_chainhead_merkle_root(source)
                .await?
                .is_some_and(|value| value == root),
            "queue root was not stored after receipt"
        );
        Ok(())
    }

    pub async fn reject_registration(
        &self,
        source: u32,
        root: Hash32,
        encoded: Vec<u8>,
    ) -> Result<()> {
        let before = self.api.read_chainhead_merkle_root(source).await?;
        let error = self
            .api
            .provide_merkle_root(source, root, encoded)
            .await
            .err()
            .context("malformed queue-root registration unexpectedly estimated")?;
        match error {
            EthereumClientError::MessageQueue(IMessageQueueErrors::InvalidPlonkProof(_)) => {}
            other => {
                bail!("malformed queue-root registration returned unexpected error: {other:?}")
            }
        }
        ensure!(
            self.api.read_chainhead_merkle_root(source).await? == before,
            "rejected queue-root registration changed state"
        );
        Ok(())
    }

    pub async fn deliver(
        &mut self,
        source: u32,
        message: &Message,
        inclusion: &MerkleProof,
    ) -> Result<()> {
        ensure!(
            message.destination == self.receiver_address(),
            "message destination is not the deployed receiver"
        );
        let guard = self.api.reserve_submission().await;
        let (tx_hash, account_nonce) = self
            .api
            .provide_content_message(
                &guard,
                source,
                u32::try_from(inclusion.num_leaves).context("message leaf count overflows u32")?,
                u32::try_from(inclusion.leaf_index).context("message leaf index overflows u32")?,
                message.nonce_be,
                message.source,
                message.destination,
                message.payload.clone(),
                inclusion.proof.clone(),
                None,
            )
            .await
            .map_err(|error| anyhow::anyhow!("submit message: {error:?}"))?;
        drop(guard);
        let receipt = self.mined_receipt(tx_hash).await?;
        ensure!(receipt.status(), "message transaction reverted");
        self.record_receipt("processMessage", &receipt, Some(account_nonce));
        ensure!(
            self.is_processed(message.nonce_be).await?,
            "message receipt did not mark nonce processed"
        );
        self.assert_message_processed(&receipt, source, message)?;
        self.assert_message_handled(&receipt, message)?;
        if let Some(transaction) = self.transactions.last_mut() {
            transaction["checkedEvents"] = json!(["MessageProcessed", "MessageHandled"]);
        }
        Ok(())
    }

    pub async fn reject_early(
        &self,
        source: u32,
        message: &Message,
        inclusion: &MerkleProof,
    ) -> Result<()> {
        self.expect_message_error(source, message, inclusion, false, false)
            .await
    }

    pub async fn reject_replay(
        &self,
        source: u32,
        message: &Message,
        inclusion: &MerkleProof,
    ) -> Result<()> {
        self.expect_message_error(source, message, inclusion, true, true)
            .await
    }

    pub async fn advance_time(&self) -> Result<()> {
        ensure!(
            self._anvil.is_some(),
            "advance_time is only available on Anvil"
        );
        let provider = self.api.raw_provider();
        let _: Value = provider
            .raw_request("evm_increaseTime".into(), (300u64,))
            .await
            .context("advance Anvil time")?;
        let _: Value = provider
            .raw_request("evm_mine".into(), ())
            .await
            .context("mine Anvil maturity block")?;
        Ok(())
    }

    pub async fn wait_maturity(&self, source: u32) -> Result<()> {
        let queue = ethereum_client::abi::IMessageQueue::new(
            self.queue_address,
            self.api.raw_provider().clone(),
        );
        let root = queue
            .getMerkleRoot(U256::from(source))
            .call()
            .await
            .context("read queue root for maturity")?;
        ensure!(root != B256::ZERO, "queue root is not registered");
        let root_timestamp = u64::try_from(
            queue
                .getMerkleRootTimestampForBlock(U256::from(source))
                .call()
                .await
                .context("read queue root timestamp")?,
        )
        .context("queue root timestamp overflows u64")?;
        ensure!(root_timestamp != 0, "queue root has no on-chain timestamp");
        let delay = u64::try_from(
            queue
                .PROCESS_USER_MESSAGE_DELAY()
                .call()
                .await
                .context("read queue user message delay")?,
        )
        .context("queue user message delay overflows u64")?;
        let ready_at = root_timestamp
            .checked_add(delay)
            .context("queue maturity timestamp overflows")?;
        loop {
            let latest = self
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Latest)
                .await
                .context("read latest Ethereum block for maturity")?
                .context("latest Ethereum block is missing")?;
            if latest.header.timestamp >= ready_at {
                return Ok(());
            }
            sleep(Duration::from_secs(12)).await;
        }
    }

    pub async fn wait_finalized(&self) -> Result<()> {
        let target_block = self
            .api
            .raw_provider()
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await
            .context("pin latest Ethereum block for finality")?
            .context("latest Ethereum block is missing")?;
        let target = target_block.header.number;
        let target_hash = target_block.header.hash;
        loop {
            let finalized = self
                .api
                .finalized_block_number()
                .await
                .context("read finalized Ethereum block")?;
            if finalized < target {
                sleep(Duration::from_secs(12)).await;
                continue;
            }
            let finalized_block = self
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Number(finalized))
                .await
                .context("read canonical finalized Ethereum block")?
                .context("finalized Ethereum block is missing")?;
            ensure!(
                finalized_block.header.number == finalized,
                "finalized Ethereum block number mismatch"
            );
            let canonical_target = self
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Number(target))
                .await
                .context("re-read finalized target Ethereum block")?
                .context("finalized target Ethereum block is missing")?;
            ensure!(
                canonical_target.header.hash == target_hash,
                "pinned Ethereum finality target changed canonical hash"
            );
            self.verify_recorded_receipts(finalized).await?;
            let checkpoint = self
                .checkpoint()
                .await
                .context("reconcile current destination checkpoint after finality")?;
            ensure!(
                checkpoint.block > 0
                    && checkpoint.current_len > 0
                    && checkpoint.next_len > 0
                    && checkpoint.current_id.checked_add(1) == Some(checkpoint.next_id),
                "current destination checkpoint is invalid"
            );
            return Ok(());
        }
    }

    async fn verify_recorded_receipts(&self, finalized: u64) -> Result<()> {
        for transaction in &self.transactions {
            let hash = transaction
                .get("txHash")
                .and_then(Value::as_str)
                .context("recorded Ethereum transaction has no hash")?;
            let hash = B256::from_str(hash).context("decode recorded Ethereum transaction hash")?;
            let receipt = self
                .api
                .raw_provider()
                .get_transaction_receipt(hash)
                .await
                .with_context(|| format!("read recorded Ethereum receipt {hash:?}"))?
                .with_context(|| format!("recorded Ethereum transaction {hash:?} is not mined"))?;
            ensure!(receipt.status(), "recorded Ethereum transaction reverted");
            let block_number = receipt
                .block_number
                .context("mined Ethereum receipt has no block number")?;
            let block_hash = receipt
                .block_hash
                .context("mined Ethereum receipt has no block hash")?;
            ensure!(
                block_number <= finalized,
                "recorded Ethereum receipt is newer than finalized head"
            );
            let canonical = self
                .api
                .raw_provider()
                .get_block_by_number(BlockNumberOrTag::Number(block_number))
                .await
                .context("read canonical Ethereum receipt block")?
                .context("canonical Ethereum receipt block is missing")?;
            ensure!(
                canonical.header.hash == block_hash,
                "recorded Ethereum receipt is not canonical"
            );
        }
        Ok(())
    }

    pub async fn is_processed(&self, nonce: Hash32) -> Result<bool> {
        ethereum_client::abi::IMessageQueue::new(
            self.queue_address,
            self.api.raw_provider().clone(),
        )
        .isProcessed(U256::from_be_bytes(nonce))
        .call()
        .await
        .context("read receipt-time Anvil processed state")
    }

    async fn expect_message_error(
        &self,
        source: u32,
        message: &Message,
        inclusion: &MerkleProof,
        processed: bool,
        replay: bool,
    ) -> Result<()> {
        ensure!(
            self.is_processed(message.nonce_be).await? == processed,
            "unexpected message processed state before rejected submission"
        );
        let guard = self.api.reserve_submission().await;
        let result = self
            .api
            .provide_content_message(
                &guard,
                source,
                u32::try_from(inclusion.num_leaves).context("message leaf count overflows u32")?,
                u32::try_from(inclusion.leaf_index).context("message leaf index overflows u32")?,
                message.nonce_be,
                message.source,
                message.destination,
                message.payload.clone(),
                inclusion.proof.clone(),
                None,
            )
            .await;
        drop(guard);
        let error = result
            .err()
            .context("rejected message unexpectedly estimated")?;
        match error.error {
            ethereum_client::Error::MessageQueue(IMessageQueueErrors::MessageAlreadyProcessed(
                _,
            )) if replay => {}
            ethereum_client::Error::MessageQueue(
                IMessageQueueErrors::MerkleRootDelayNotPassed(_),
            ) if !replay => {}
            other => bail!("rejected message returned unexpected error: {other:?}"),
        }
        ensure!(
            self.is_processed(message.nonce_be).await? == processed,
            "rejected message changed processed state"
        );
        Ok(())
    }

    fn record_receipt(
        &mut self,
        label: &str,
        receipt: &alloy::rpc::types::TransactionReceipt,
        account_nonce: Option<u64>,
    ) {
        self.transactions
            .push(receipt_value(label, receipt, account_nonce));
    }

    fn assert_message_processed(
        &self,
        receipt: &alloy::rpc::types::TransactionReceipt,
        source: u32,
        message: &Message,
    ) -> Result<()> {
        let signature = B256::from(keccak256(
            b"MessageProcessed(uint256,bytes32,uint256,address)",
        ));
        let expected_hash = B256::from(crate::message_hash(message));
        let expected_nonce = U256::from_be_slice(&message.nonce_be);
        for log in receipt.as_ref().logs() {
            if log.address() != self.queue_address || log.topic0() != Some(&signature) {
                continue;
            }
            ensure!(
                log.topics().len() == 1,
                "MessageProcessed log has unexpected topics"
            );
            let data = log.data().data.as_ref();
            ensure!(
                data.len() == 128,
                "MessageProcessed data is not canonical ABI"
            );
            let block_number = U256::from_be_slice(&data[..32]);
            let message_hash = B256::from_slice(&data[32..64]);
            let message_nonce = U256::from_be_slice(&data[64..96]);
            let message_destination = Address::from_slice(&data[108..128]);
            ensure!(
                block_number == U256::from(source),
                "MessageProcessed block number mismatch"
            );
            ensure!(
                message_hash == expected_hash,
                "MessageProcessed hash mismatch"
            );
            ensure!(
                message_nonce == expected_nonce,
                "MessageProcessed nonce mismatch"
            );
            ensure!(
                message_destination == message.destination,
                "MessageProcessed destination mismatch"
            );
            return Ok(());
        }
        bail!("receipt did not contain exact MessageProcessed event")
    }
    fn assert_message_handled(
        &self,
        receipt: &alloy::rpc::types::TransactionReceipt,
        message: &Message,
    ) -> Result<()> {
        let signature = B256::from(keccak256(b"MessageHandled(bytes32,bytes)"));
        for log in receipt.as_ref().logs() {
            if log.address() != self.receiver_address || log.topic0() != Some(&signature) {
                continue;
            }
            let topics = log.topics();
            ensure!(
                topics.len() == 2,
                "MessageHandled log has unexpected topics"
            );
            ensure!(
                topics[1].as_slice() == message.source,
                "MessageHandled source mismatch"
            );
            let data = log.data().data.as_ref();
            ensure!(
                data.len() >= 64,
                "MessageHandled payload encoding is truncated"
            );
            let offset = U256::from_be_slice(&data[..32]);
            ensure!(
                offset == U256::from(32u64),
                "MessageHandled payload offset is noncanonical"
            );
            let length = usize::try_from(U256::from_be_slice(&data[32..64]))
                .context("MessageHandled payload length overflows usize")?;
            ensure!(
                data.len() >= 64 + length,
                "MessageHandled payload is truncated"
            );
            ensure!(
                data[64..64 + length] == message.payload,
                "MessageHandled payload mismatch"
            );
            return Ok(());
        }
        bail!("receipt did not contain exact MessageHandled event")
    }
}

fn hoodi_ws_endpoint(endpoint: &str) -> Result<String> {
    let endpoint = if endpoint.is_empty() {
        HOODI_DEFAULT_RPC.to_owned()
    } else if let Some(rest) = endpoint.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = endpoint.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        endpoint.to_owned()
    };
    ensure!(
        endpoint.starts_with("ws://") || endpoint.starts_with("wss://"),
        "Hoodi Ethereum endpoint must use ws:// or wss://"
    );
    Ok(endpoint)
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

async fn ensure_hoodi_network(endpoint: &str) -> Result<()> {
    let provider = PollingEthApi::new(endpoint)
        .await
        .context("connect to Hoodi Ethereum endpoint")?;
    ensure!(
        provider.get_chain_id().await? == HOODI_CHAIN_ID,
        "Hoodi Ethereum endpoint has unexpected chain id"
    );
    let genesis = provider
        .get_block_by_number(BlockNumberOrTag::Number(0))
        .await
        .context("read Hoodi genesis block")?
        .context("Hoodi genesis block is missing")?;
    let expected = B256::from_str(HOODI_GENESIS_BLOCK).expect("valid Hoodi genesis constant");
    ensure!(
        genesis.header.hash == expected,
        "Hoodi Ethereum endpoint has unexpected genesis block"
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
    let expected_domain = beefy_relay::bridge_domain(
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

async fn verify_hoodi_bootstrap(
    ethereum: &Ethereum,
    checkpoint: &CapturedCommitment,
    snapshot: &QueueSnapshot,
    mmr_start_block: u64,
) -> Result<()> {
    let deployed = ethereum.checkpoint().await?;
    ensure!(
        deployed.block == u64::from(checkpoint.block) && deployed.root == [0; 32],
        "Hoodi bootstrap client did not preserve initial block and zero accepted root"
    );
    ensure!(
        deployed.source_timestamp_ms == snapshot.source_timestamp_ms,
        "Hoodi bootstrap source timestamp was not authenticated"
    );
    ensure!(
        deployed.current_id == checkpoint.current.id
            && deployed.current_len == checkpoint.current.keys.len() as u64
            && deployed.current_root == checkpoint.current.root
            && deployed.next_id == checkpoint.next.id
            && deployed.next_len == checkpoint.next.keys.len() as u64
            && deployed.next_root == checkpoint.next.root,
        "Hoodi deployment authority checkpoint does not match trusted source checkpoint"
    );
    let provider = ethereum.api.raw_provider().clone();
    let client = BeefyClient::new(ethereum.client_address, provider.clone());
    ensure!(
        client.bridgeDomain().call().await? == B256::from(snapshot.bridge_domain)
            && client.mmrStartBlock().call().await? == mmr_start_block,
        "Hoodi deployed source identity does not match bootstrap"
    );
    verify_secure_policy(ethereum.client_address, provider.clone()).await?;
    verify_bindings(
        ethereum.verifier_address,
        ethereum.client_address,
        ethereum.queue_address,
        HOODI_CHAIN_ID,
        provider,
    )
    .await
}

async fn verify_hoodi_manifest(ethereum: &Ethereum, manifest: &Value) -> Result<()> {
    let provider = ethereum.api.raw_provider().clone();
    ensure!(
        provider.get_chain_id().await? == HOODI_CHAIN_ID,
        "connected Ethereum endpoint has unexpected Hoodi chain id"
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
        HOODI_CHAIN_ID,
        provider.clone(),
    )
    .await?;
    let recovery_controller = manifest_address(manifest, "recoveryController")?;
    let recovery_wallet = manifest_address(manifest, "recoveryWallet")?;
    ensure!(
        TokenQueueBinding::new(ethereum.queue_address, provider.clone())
            .recoveryController()
            .call()
            .await?
            == recovery_controller,
        "recovery controller differs from immutable deployment identity"
    );
    let recovery = TokenRecoveryControllerBinding::new(recovery_controller, provider.clone());
    ensure!(
        recovery.messageQueue().call().await? == ethereum.queue_address
            && recovery.recoveryWallet().call().await? == recovery_wallet
            && recovery.RECOVERY_DELAY().call().await? == U256::from(24 * 60 * 60u64),
        "recovery wallet/controller binding or 24-hour delay differs from deployment identity"
    );
    for address in [recovery_controller, recovery_wallet] {
        ensure!(
            !provider.get_code_at(address).await?.is_empty(),
            "pinned recovery controller/wallet has no deployed code"
        );
    }
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
fn address_string(address: Address) -> String {
    format!("0x{}", hex::encode(address.as_slice()))
}

fn parse_address(value: &Value) -> Option<Address> {
    if let Some(value) = value.as_str() {
        return Address::from_str(value).ok();
    }
    value.get("value").and_then(parse_address)
}

fn return_address(returns: &Value, name: &str) -> Option<Address> {
    returns.as_object()?.get(name).and_then(parse_address)
}

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
mod tests {
    use super::*;
    use alloy::{
        consensus::{SignableTransaction, TxEip1559, TxLegacy},
        network::TxSigner,
        primitives::TxKind,
    };

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
