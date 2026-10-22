//! Test-only Beacon endpoint: real POP signatures and SSZ branches, never a verifier bypass.
//! Uses public deterministic key 1 (the existing signature-boundary fixture's first key).
use super::*;
use builtins_common::bls12_381::{ark_bls12_381::{Fr, G1Projective, G2Affine}, ark_ec::{AffineRepr, Group}, ark_serialize::CanonicalSerialize};
use serde_json::{json, Value};
use std::{collections::BTreeMap, future::Future};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[path = "beacon_scenario_fixture.rs"]
mod scenario;
pub use scenario::export_scenario;

/// A genuine single-account state trie and matching `eth_getProof` response for test fixtures.
pub fn single_account_proof(
    address: alloy::primitives::Address,
    nonce: u64,
) -> (B256, alloy::rpc::types::EIP1186AccountProofResponse) {
    use ethereum_common::{
        memory_db, patricia_trie::{TrieDB, TrieDBMut}, trie_db::{Recorder, Trie, TrieMut},
    };
    let account = alloy::consensus::TrieAccount { nonce, ..Default::default() };
    let encoded = alloy::rlp::encode(account);
    let key = alloy::primitives::keccak256(address);
    let mut database = memory_db::new();
    let mut root = ethereum_common::H256::zero();
    {
        let mut trie = TrieDBMut::new(&mut database, &mut root);
        trie.insert(key.as_slice(), &encoded).expect("single fixture account inserts");
        let _ = trie.root();
    }
    let trie = TrieDB::new(&database, &root).expect("fixture account trie exists");
    let mut recorder = Recorder::new();
    assert_eq!(
        trie.get_with(key.as_slice(), &mut recorder).expect("fixture account is readable").as_deref(),
        Some(encoded.as_slice()),
    );
    (B256::from(root.0), alloy::rpc::types::EIP1186AccountProofResponse {
        address, nonce: account.nonce, balance: account.balance,
        code_hash: account.code_hash, storage_hash: account.storage_root,
        account_proof: recorder.drain().into_iter().map(|node| node.data.into()).collect(),
        storage_proof: Vec::new(),
    })
}

type HeadSource = Arc<dyn Fn() -> std::pin::Pin<Box<dyn Future<Output = Result<Header>> + Send>> + Send + Sync>;

pub struct LocalBeaconFixture {
    pub config: BeaconFinalityConfig,
    pub directory: PathBuf,
    pub approval_path: PathBuf,
    pub approval_sha256: B256,
    pub beacon_rpc: String,
    server: tokio::task::JoinHandle<()>,
    generator: Arc<std::sync::Mutex<Generator>>,
    head: HeadSource,
    bootstrap: Value,
    initial_update: Value,
}
impl Drop for LocalBeaconFixture {
    fn drop(&mut self) {
        self.server.abort();
        let _ = fs::remove_dir_all(&self.directory);
    }
}
fn hex(bytes: impl AsRef<[u8]>) -> String { format!("0x{}", alloy::hex::encode(bytes)) }
fn header_json(header: &BeaconBlockHeader) -> Value {
    json!({"slot":header.slot.to_string(),"proposer_index":header.proposer_index.to_string(),
        "parent_root":hex(header.parent_root),"state_root":hex(header.state_root),"body_root":hex(header.body_root)})
}
fn state_tree(committee: [u8;32], finalized: [u8;32]) -> (B256, Vec<[u8;32]>, Vec<[u8;32]>, Vec<[u8;32]>) {
    let mut nodes = vec![[0;32];256];
    nodes[86] = committee;
    nodes[87] = committee;
    nodes[169] = finalized;
    for index in (1..128).rev() {
        if index == 86 || index == 87 { continue; }
        let mut bytes = [0;64];
        bytes[..32].copy_from_slice(&nodes[index*2]);
        bytes[32..].copy_from_slice(&nodes[index*2+1]);
        nodes[index] = digest(&bytes).0;
    }
    let branch = |mut index: usize| {
        let mut proof = Vec::new();
        while index > 1 { proof.push(nodes[index ^ 1]); index /= 2; }
        proof
    };
    (B256::from(nodes[1]), branch(86), branch(87), branch(169))
}
struct SignedBlock {
    bootstrap: Value,
    update: Value,
    header: BeaconBlockHeader,
}
struct Generator {
    network: Network,
    template: Value,
    committee: Value,
    committee_root: [u8;32],
    blocks: BTreeMap<[u8;32], Value>,
}
impl Generator {
    fn new(network: Network) -> Self {
        let mut public = Vec::new();
        G1Projective::generator().serialize_compressed(&mut public).unwrap();
        let mut aggregate = Vec::new();
        (G1Projective::generator() * Fr::from(512u64)).serialize_compressed(&mut aggregate).unwrap();
        let committee = json!({"pubkeys":vec![hex(&public);512], "aggregate_pubkey":hex(aggregate)});
        let decoded: beacon::SyncCommittee = serde::Deserialize::deserialize(&committee).unwrap();
        let template: Value = serde_json::from_slice(include_bytes!("../../../tests/src/checkpoint_light_client/chain-data/mainnet-electra-11649029-beacon-block.json")).unwrap();
        let mut template = template["data"]["message"].clone();
        // LocalFixture retains the complete Electra shape, not unrelated mainnet list contents.
        for field in ["proposer_slashings", "attester_slashings", "attestations", "deposits",
            "voluntary_exits", "bls_to_execution_changes", "blob_kzg_commitments"] {
            template["body"][field] = json!([]);
        }
        for field in ["transactions", "withdrawals"] {
            template["body"]["execution_payload"][field] = json!([]);
        }
        for field in ["deposits", "withdrawals", "consolidations"] {
            template["body"]["execution_requests"][field] = json!([]);
        }
        Self { network, template, committee,
            committee_root: decoded.tree_hash_root().0, blocks: BTreeMap::new() }
    }
    fn signed(&mut self, execution: &Header, slot: u64) -> SignedBlock {
        self.signed_payload(execution, slot, &[])
    }

    fn signed_payload(&mut self, execution: &Header, slot: u64, transactions: &[String]) -> SignedBlock {
        let (bootstrap_state, current_branch, _, _) = state_tree(self.committee_root, [0;32]);
        let mut block = self.template.clone();
        block["slot"] = json!(slot.to_string());
        block["state_root"] = json!(bootstrap_state);
        block["body"]["execution_payload"]["block_number"] = json!(execution.number.to_string());
        block["body"]["execution_payload"]["block_hash"] = json!(execution.hash);
        let payload = &mut block["body"]["execution_payload"];
        payload["parent_hash"] = json!(execution.parent_hash);
        payload["fee_recipient"] = json!(execution.beneficiary);
        payload["state_root"] = json!(execution.state_root);
        payload["receipts_root"] = json!(execution.receipts_root);
        payload["logs_bloom"] = json!(execution.logs_bloom);
        payload["prev_randao"] = json!(execution.mix_hash);
        payload["gas_limit"] = json!(execution.gas_limit.to_string());
        payload["gas_used"] = json!(execution.gas_used.to_string());
        payload["timestamp"] = json!(execution.timestamp.to_string());
        payload["extra_data"] = json!(execution.extra_data);
        payload["base_fee_per_gas"] = json!(execution.base_fee_per_gas.unwrap_or_default().to_string());
        payload["blob_gas_used"] = json!(execution.blob_gas_used.unwrap_or_default().to_string());
        payload["excess_blob_gas"] = json!(execution.excess_blob_gas.unwrap_or_default().to_string());
        payload["transactions"] = json!(transactions);
        let decoded: beacon::electra::Block = serde::Deserialize::deserialize(&block).unwrap();
        let header = BeaconBlockHeader { slot, proposer_index: decoded.proposer_index,
            parent_root: decoded.parent_root, state_root: decoded.state_root,
            body_root: decoded.body.tree_hash_root() };
        let (state_root, _, next_branch, finality_branch) = state_tree(self.committee_root, header.tree_hash_root().0);
        let attested = BeaconBlockHeader { slot: slot+1, state_root: state_root.0.into(), ..header.clone() };
        let domain = ethereum_common::signing_root::compute_domain(ethereum_common::DOMAIN_SYNC_COMMITTEE,
            self.network.fork_version(slot+1), self.network.genesis_validators_root());
        let signing = ethereum_common::signing_root::compute(attested.tree_hash_root(), domain);
        let encoded = Bls12_381OpsLowLevel::map_to_g2affine(signing.as_ref().to_vec()).unwrap();
        let point = checkpoint_light_client_io::ArkScale::<G2Affine>::decode_all(&mut &encoded[..]).unwrap().0;
        let mut signature = Vec::new();
        (point.into_group() * Fr::from(512u64)).serialize_compressed(&mut signature).unwrap();
        let bootstrap = json!({"data":{"header":{"beacon":header_json(&header)},
            "current_sync_committee":self.committee,"current_sync_committee_branch":current_branch.into_iter().map(hex).collect::<Vec<_>>()}});
        let update = json!({"data":{"attested_header":{"beacon":header_json(&attested)},
            "finalized_header":{"beacon":header_json(&header)},"next_sync_committee":self.committee,
            "next_sync_committee_branch":next_branch.into_iter().map(hex).collect::<Vec<_>>(),
            "finality_branch":finality_branch.into_iter().map(hex).collect::<Vec<_>>(),
            "sync_aggregate":{"sync_committee_bits":hex([255;64]),"sync_committee_signature":hex(signature)},
            "signature_slot":(slot+2).to_string()}});
        self.blocks.insert(header.tree_hash_root().0, block);
        SignedBlock { bootstrap, update, header }
    }
}
impl LocalBeaconFixture {
    pub async fn new(chain_id: u64, genesis: B256, head: Arc<dyn Fn() -> Header + Send + Sync>) -> Result<Self> {
        Self::with_head(chain_id, genesis, Arc::new(move || {
            let header = head();
            Box::pin(std::future::ready(Ok(header)))
        })).await
    }

    /// Follow an execution provider asynchronously; suitable for current-thread test runtimes.
    pub async fn from_provider(
        provider: alloy::providers::RootProvider,
        tag: alloy::rpc::types::BlockNumberOrTag,
    ) -> Result<Self> {
        let chain_id = provider.get_chain_id().await?;
        ensure!(chain_id == 31337, "signed fixture is restricted to chain 31337");
        let genesis = provider.get_block_by_number(0.into()).await?
            .context("fixture provider omitted execution genesis")?.header.hash;
        Self::with_head(chain_id, genesis, Arc::new(move || {
            let provider = provider.clone();
            Box::pin(async move {
                Ok(provider.get_block_by_number(tag).await?
                    .context("fixture provider omitted requested execution head")?.header)
            })
        })).await
    }

    async fn with_head(chain_id: u64, genesis: B256, head: HeadSource) -> Result<Self> {
        ensure!(chain_id == 31337, "signed fixture is restricted to chain 31337");
        // Explicit test-only approval window; not a production weak-subjectivity default.
        let network = Network::LocalFixture { genesis_time: 1_700_000_000,
            genesis_validators_root: [101;32].into() };
        let mut generator = Generator::new(network.clone());
        let initial = head().await?;
        ensure!(initial.number < 240_000, "signed fixture height exceeds its bounded clock range");
        let initial_slot = 8192 + initial.number * 4;
        let bootstrap = generator.signed(&initial, initial_slot);
        let trusted_root = B256::from(bootstrap.header.tree_hash_root().0);
        let initial_update = bootstrap.update.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}", listener.local_addr()?);
        let generator = Arc::new(std::sync::Mutex::new(generator));
        let fixture_generator = generator.clone();
        let fixture_head = head.clone();
        let fixture_bootstrap = bootstrap.bootstrap.clone();
        let fixture_initial_update = initial_update.clone();
        let approval = BootstrapApproval { schema_version:1, chain_id, genesis_hash:genesis,
            genesis_validators_root:B256::from(network.genesis_validators_root().0), genesis_time:network.genesis_time(),network,
            trusted_bootstrap_root:trusted_root,trusted_bootstrap_slot:initial_slot,
            approved_at_ms:1_700_000_000_000,bootstrap_expires_at_ms:2_000_000_000_000 };
        let bytes = serde_json::to_vec(&approval)?;
        let approval_sha256 = digest(&bytes);
        let config = BeaconFinalityConfig::parse(bytes.clone(), approval_sha256, &url)?;
        let directory = std::env::temp_dir().join(format!("signed-beacon-{}-{}",std::process::id(),SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()));
        fs::create_dir_all(&directory)?;
        let approval_path = directory.join("fixture-approval.json");
        fs::write(&approval_path, &bytes)?;
        let server = tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else { break };
                let generator = generator.clone();
                let head = head.clone();
                let bootstrap = bootstrap.bootstrap.clone();
                let initial_update = initial_update.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    loop {
                        let mut chunk = [0;4096];
                        let Ok(count) = socket.read(&mut chunk).await else { return };
                        if count == 0 { return; }
                        request.extend_from_slice(&chunk[..count]);
                        if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") { break; }
                    }
                    let response: Result<Value> = async {
                        let request = std::str::from_utf8(&request)?;
                        let route = request.split_whitespace().nth(1).context("missing fixture route")?;
                        if route.starts_with("/eth/v1/beacon/light_client/bootstrap/") {
                            Ok(bootstrap)
                        } else if route.starts_with("/eth/v2/beacon/blocks/0x") {
                            let root: B256 = route.rsplit('/').next().context("missing Beacon root")?.parse()?;
                            let generator = generator.lock().map_err(|_| anyhow::anyhow!("fixture generator poisoned"))?;
                            Ok(json!({"data":{"message":generator.blocks.get(&root.0).context("unknown fixture Beacon root")?}}))
                        } else {
                            let head = head().await?;
                            ensure!(head.number < 240_000, "signed fixture height exceeds its bounded clock range");
                            let slot = 8192 + head.number * 4;
                            let mut generator = generator.lock().map_err(|_| anyhow::anyhow!("fixture generator poisoned"))?;
                            if route.contains("light_client/updates?") {
                                let period: u64 = route.split("start_period=").nth(1).context("missing start period")?
                                    .split('&').next().context("missing start period")?.parse()?;
                                ensure!(period < 1000, "fixture period out of range");
                                if period == utils::calculate_period(initial_slot) && head.hash == initial.hash {
                                    Ok(json!([initial_update]))
                                } else {
                                    let selected = slot.min(utils::calculate_slot(period + 1)-4);
                                    Ok(json!([generator.signed(&head, selected).update]))
                                }
                            } else { Ok(generator.signed(&head, slot).update) }
                        }
                    }.await;
                    let (status, response) = match response {
                        Ok(value) => ("200 OK", value),
                        Err(error) => ("503 Service Unavailable", json!({"error":error.to_string()})),
                    };
                    let response = response.to_string();
                    let _ = socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}", response.len()).as_bytes()).await;
                });
            }
        });
        Ok(Self { config, directory, approval_path, approval_sha256, beacon_rpc: url,
            server, generator: fixture_generator, head: fixture_head,
            bootstrap: fixture_bootstrap, initial_update: fixture_initial_update })
    }

    /// Export exact normal Beacon/EL RPC responses for native-child and JS database tests.
    /// The caller supplies full execution blocks and receipts; inconsistent MPT roots fail.
    pub async fn export_bundle(
        &self,
        path: impl AsRef<Path>,
        execution_blocks: &[alloy::rpc::types::Block],
        receipts: &[alloy::rpc::types::TransactionReceipt],
    ) -> Result<B256> {
        let mut bundle = execution_bundle(execution_blocks, receipts, self.config.identity().1)?;
        let head = (self.head)().await?;
        ensure!(execution_blocks.last().is_some_and(|block| block.header.inner == head.inner && block.header.hash == head.hash),
            "fixture execution tip differs from signed Beacon head");
        let mut generator = self.generator.lock().unwrap();
        let finality = generator.signed(&head, 8192 + head.number * 4);
        let blocks = generator.blocks.iter().map(|(root, block)| (hex(root), json!({"data":{"message":block}}))).collect::<BTreeMap<_,_>>();
        bundle.as_object_mut().unwrap().extend(json!({
            "schemaVersion":1, "description":"Deterministic signed LocalFixture chain31337; NOT public-network proof or production approval",
            "approvalFileBytes":self.config.approval_bytes, "approvalSha256":self.config.approval_hash,
            "bootstrap":self.bootstrap, "updates":[self.initial_update], "finalityUpdate":finality.update,
            "beaconBlocksByRoot":blocks,
        }).as_object().unwrap().clone());
        let bytes = serde_json::to_vec(&bundle)?;
        gear_common::durable::immutable_json(path.as_ref(), &bundle)?;
        Ok(digest(&bytes))
    }
}

/// Shared canonical execution validation and inclusion-proof construction for both exporters.
fn execution_bundle(
    execution_blocks: &[alloy::rpc::types::Block],
    receipts: &[alloy::rpc::types::TransactionReceipt],
    genesis: B256,
) -> Result<Value> {
        use alloy::eips::Encodable2718;
        let first = execution_blocks.first().context("fixture needs its execution genesis")?;
        ensure!(first.header.number == 0 && first.header.hash == genesis,
            "fixture execution genesis differs from approval");
        ensure!(execution_blocks.windows(2).all(|pair| pair[0].header.number.checked_add(1) == Some(pair[1].header.number)
            && pair[1].header.parent_hash == pair[0].header.hash), "fixture execution ancestry is incomplete");
        let mut proofs = Vec::new();
        let mut used_receipts = 0usize;
        for block in execution_blocks {
            ensure!(block.header.inner.hash_slow() == block.header.hash, "fixture execution RLP hash mismatch");
            let transactions = block.transactions.clone().try_into_transactions()
                .map_err(|_| anyhow::anyhow!("fixture export requires complete transactions"))?
                .into_iter().map(|transaction| transaction.into_inner()).collect::<Vec<_>>();
            ensure!(alloy::consensus::proofs::calculate_transaction_root(&transactions) == block.header.transactions_root,
                "fixture transaction root mismatch");
            let mut block_receipts = receipts.iter().filter(|receipt| receipt.block_hash == Some(block.header.hash)).cloned().collect::<Vec<_>>();
            block_receipts.sort_by_key(|receipt| receipt.transaction_index);
            ensure!(block_receipts.len() == transactions.len(), "fixture receipt count mismatch");
            let mut log_index = 0;
            for (index, receipt) in block_receipts.iter().enumerate() {
                ensure!(receipt.transaction_index == Some(index as u64) && receipt.block_number == Some(block.header.number)
                    && receipt.transaction_hash == *transactions[index].tx_hash(), "fixture receipt coordinates mismatch");
                for log in receipt.inner.logs() {
                    ensure!(!log.removed && log.block_number == Some(block.header.number) && log.block_hash == Some(block.header.hash)
                        && log.transaction_index == Some(index as u64) && log.transaction_hash == Some(receipt.transaction_hash)
                        && log.log_index == Some(log_index), "fixture log coordinates mismatch");
                    log_index += 1;
                }
            }
            used_receipts += block_receipts.len();
            let envelopes = block_receipts.into_iter().enumerate().map(|(index, receipt)| (index as u64, receipt.into_primitives_receipt().inner)).collect::<Vec<_>>();
            ensure!(alloy::consensus::proofs::calculate_receipt_root(&envelopes.iter().map(|(_, receipt)| receipt).collect::<Vec<_>>()) == block.header.receipts_root,
                "fixture receipt root mismatch");
            for (index, transaction) in transactions.iter().enumerate() {
                let receipt = utils::generate_merkle_proof(index as u64, &envelopes)?;
                let proof = crate::inclusion::RecordedReceiptProof { genesis: first.header.hash,
                    header: block.header.inner.clone(), index: index as u64, raw_transaction: transaction.encoded_2718(),
                    receipt: receipt.receipt,
                    transaction_proof: crate::inclusion::transaction_proof(&transactions, index as u64, block.header.transactions_root)?,
                    receipt_proof: receipt.proof };
                crate::inclusion::verify_indexed_value(proof.header.receipts_root, proof.index, &proof.receipt.encoded_2718(), &proof.receipt_proof)?;
                proofs.push(proof);
            }
        }
        ensure!(used_receipts == receipts.len(), "fixture includes receipts outside authenticated blocks");
    Ok(json!({ "executionBlocks": execution_blocks, "receipts": receipts, "inclusionProofs": proofs }))
}
