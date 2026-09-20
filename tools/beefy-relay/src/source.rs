use anyhow::{anyhow, bail, ensure, Context, Result};
use gsdk::{
    ext::{
        sp_core::{sr25519, Pair as GearPair},
        subxt::utils::H256,
    },
    PairSigner, Value,
};
use parity_scale_codec::Decode;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use sp_consensus_beefy::{
    ecdsa_crypto::{AuthorityId, Pair as BeefyPair},
    mmr::BeefyAuthoritySet,
    ConsensusLog, ValidatorSet,
};
use sp_core::Pair as BeefyPairTrait;
use sp_runtime::generic::DigestItem;
use subxt_rpcs::{client::RpcSubscription, rpc_params};
use tokio::time::{timeout, Duration, Instant};

use gear_rpc_client::{dto, GearApi};

use beefy_relay::{
    authority_addresses, authority_root, convert_mmr_proof, decode_outer_leaf,
    decode_versioned_finality_proof, keccak256, outer_leaf_hash, validate_mmr_coordinates,
    validate_signed_commitment, verify_native_mmr_proof, Hash32, QueueSnapshot, RuntimeLeaf,
    RuntimeLeafProof, SimplifiedMmrProof, ValidatedCommitment, SNAPSHOT_VERSION_UNINITIALIZED,
};

const MAX_WAIT: Duration = Duration::from_secs(45);
const BEEFY_ENGINE: [u8; 4] = *b"BEEF";
const AUTHORITY_KEY_TYPE: &str = "beef";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct AuthoritySet {
    pub id: u64,
    pub keys: Vec<Vec<u8>>,
    pub root: Hash32,
}

#[derive(Clone, Debug)]
pub struct CapturedCommitment {
    pub block: u32,
    pub block_hash: Hash32,
    pub raw: Vec<u8>,
    pub validated: ValidatedCommitment,
    pub current: AuthoritySet,
    pub next: AuthoritySet,
}

#[derive(Clone, Debug)]
pub struct SourceProof {
    pub source: u32,
    pub source_hash: Hash32,
    pub snapshot: QueueSnapshot,
    pub leaf: RuntimeLeaf,
    pub raw_leaf: Vec<u8>,
    pub proof: RuntimeLeafProof,
    pub simplified: SimplifiedMmrProof,
}

#[derive(Clone, Debug, Serialize)]
pub struct RotationEvidence {
    pub authority: String,
    pub stash: Hash32,
    pub beefy_key: Vec<u8>,
    pub extrinsic_hash: Hash32,
    pub block: u32,
    pub block_hash: Hash32,
}

#[derive(Clone, Debug)]
pub struct ObservedMessage {
    pub block: u32,
    pub block_hash: Hash32,
    pub message: dto::Message,
    pub message_hash: Hash32,
    pub inclusion: dto::MerkleProof,
    pub snapshot: QueueSnapshot,
    pub retained_at_block: u32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MmrLeavesProof {
    block_hash: String,
    leaves: String,
    proof: String,
}

pub struct Source {
    pub api: GearApi,
    subscription: RpcSubscription<String>,
    pending: std::collections::BTreeMap<u32, Vec<u8>>,
    accepted: std::collections::BTreeMap<u32, Vec<u8>>,
    last_block: Option<u32>,
    last_set_id: Option<u64>,
}

impl Source {
    pub async fn connect(url: &str) -> Result<Self> {
        let api = GearApi::new(url, 3).await.context("connect Gear RPC")?;
        // Subscribe before any catch-up RPC so no signed justification can race the scan.
        let subscription = api
            .api
            .rpc()
            .subscribe(
                "beefy_subscribeJustifications",
                rpc_params![],
                "beefy_unsubscribeJustifications",
            )
            .await
            .context("subscribe to BEEFY justifications")?;
        Ok(Self {
            api,
            subscription,
            pending: Default::default(),
            accepted: Default::default(),
            last_block: None,
            last_set_id: None,
        })
    }

    pub async fn next_commitment(&mut self) -> Result<CapturedCommitment> {
        timeout(MAX_WAIT, self.next_commitment_inner())
            .await
            .map_err(|_| anyhow!("timed out waiting for a complete BEEFY commitment"))?
    }

    async fn next_commitment_inner(&mut self) -> Result<CapturedCommitment> {
        loop {
            let finalized_hash = self.api.latest_finalized_block().await?;
            let finalized = self.api.block_hash_to_number(finalized_hash).await?;
            self.scan_range(finalized).await?;

            if let Some((&block, _)) = self.pending.iter().next() {
                let raw = self.pending.remove(&block).expect("entry just observed");
                let captured = self.capture(block, raw.clone()).await?;
                self.accepted.insert(block, raw);
                self.last_block = Some(block);
                self.last_set_id = Some(captured.validated.signed.commitment.validator_set_id);
                return Ok(captured);
            }

            match timeout(Duration::from_secs(3), self.subscription.next()).await {
                Err(_) => {}
                Ok(None) => bail!("BEEFY justification subscription closed"),
                Ok(Some(Err(error))) => {
                    return Err(anyhow!("BEEFY justification subscription failed: {error}"))
                }
                Ok(Some(Ok(raw))) => {
                    self.queue_justification(decode_hex(&raw)?)?;
                }
            }
        }
    }

    async fn scan_range(&mut self, finalized: u32) -> Result<()> {
        let start = self.last_block.map_or(1, |block| block.saturating_add(1));
        if start > finalized {
            return Ok(());
        }
        for block in start..=finalized {
            let hash = self.api.block_number_to_hash(block).await?;
            let raw = self
                .api
                .api
                .rpc()
                .request::<JsonValue>("chain_getBlock", rpc_params![hash])
                .await
                .with_context(|| format!("read block {block} for BEEFY catch-up"))?;
            if let Some(bytes) = justification_bytes(&raw)? {
                self.queue_justification(bytes)?;
            }
        }
        Ok(())
    }

    fn queue_justification(&mut self, bytes: Vec<u8>) -> Result<()> {
        let signed =
            decode_versioned_finality_proof(&bytes).context("decode BEEFY justification")?;
        let block = signed.commitment.block_number;
        ensure!(block > 0, "genesis BEEFY commitment is not usable");
        if let Some(existing) = self.accepted.get(&block) {
            ensure!(
                existing == &bytes,
                "conflicting BEEFY justifications for accepted block {block}"
            );
            return Ok(());
        }
        if let Some(previous) = self.last_block {
            ensure!(
                block > previous,
                "conflicting historical BEEFY justification at block {block}"
            );
        }
        if let Some(existing) = self.pending.get(&block) {
            ensure!(
                existing == &bytes,
                "conflicting BEEFY justifications for block {block}"
            );
        } else {
            self.pending.insert(block, bytes);
        }
        Ok(())
    }

    async fn capture(&self, block: u32, raw: Vec<u8>) -> Result<CapturedCommitment> {
        if let Some(previous) = self.last_block {
            ensure!(
                block > previous,
                "BEEFY commitment history is not increasing"
            );
        }
        let (current, next) = self.checkpoint_at(block).await?;
        let current_keys = key_arrays(&current.keys)?;
        let validated =
            validate_signed_commitment(&raw, &current_keys, Some(current.id), Some(block))
                .with_context(|| format!("validate signed BEEFY commitment at block {block}"))?;
        ensure!(
            self.mmr_digest(block).await? == Some(validated.mmr_root),
            "BEEFY commitment MMR root does not match the block digest"
        );
        if let Some(last_id) = self.last_set_id {
            ensure!(
                validated.signed.commitment.validator_set_id == last_id
                    || validated.signed.commitment.validator_set_id == last_id + 1,
                "unknown or skipped BEEFY authority set {} after {}",
                validated.signed.commitment.validator_set_id,
                last_id
            );
        }
        ensure!(
            validated.signed.commitment.validator_set_id == current.id,
            "commitment set id does not match historical current authority set"
        );
        let block_hash = self.api.block_number_to_hash(block).await?;
        Ok(CapturedCommitment {
            block,
            block_hash: block_hash.0,
            raw,
            validated,
            current,
            next,
        })
    }

    async fn mmr_digest(&self, block: u32) -> Result<Option<Hash32>> {
        let hash = self.api.block_number_to_hash(block).await?;
        let block_json: JsonValue = self
            .api
            .api
            .rpc()
            .request::<JsonValue>("chain_getBlock", rpc_params![hash])
            .await?;
        let logs = block_json
            .get("block")
            .and_then(|v| v.get("header"))
            .and_then(|v| v.get("digest"))
            .and_then(|v| v.get("logs"))
            .and_then(JsonValue::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for log in logs {
            let Some(hex_log) = log.as_str() else {
                continue;
            };
            let bytes = decode_hex(hex_log)?;
            let mut input = &bytes[..];
            let item = DigestItem::decode(&mut input).context("decode block digest")?;
            ensure!(input.is_empty(), "BEEFY digest item has trailing bytes");
            if let DigestItem::Consensus(engine, payload) = item {
                if engine != BEEFY_ENGINE {
                    continue;
                }
                let mut payload_input = &payload[..];
                let consensus = ConsensusLog::<AuthorityId>::decode(&mut payload_input)
                    .context("decode BEEFY consensus digest")?;
                ensure!(
                    payload_input.is_empty(),
                    "BEEFY consensus digest has trailing bytes"
                );
                if let ConsensusLog::MmrRoot(root) = consensus {
                    return Ok(Some(root.into()));
                }
            }
        }
        Ok(None)
    }

    pub async fn checkpoint_at(&self, block: u32) -> Result<(AuthoritySet, AuthoritySet)> {
        checkpoint_at(&self.api, block).await
    }

    /// Verifies leaf and authority inclusion; queue envelopes also bind the parent snapshot.
    /// Session hooks can change bridge state before a handover leaf is inserted.
    pub async fn proof(&self, source: u32, anchor: &CapturedCommitment) -> Result<SourceProof> {
        ensure!(
            source < anchor.block,
            "source block must precede BEEFY anchor"
        );
        let source_hash = self.api.block_number_to_hash(source).await?.0;
        let anchor_hash = H256(anchor.block_hash);
        let snapshot = if bridge_initialized(&self.api, H256(source_hash)).await? {
            let (queue_id, queue_root) =
                self.api.fetch_queue_merkle_root(H256(source_hash)).await?;
            QueueSnapshot::new(0, queue_id, queue_root.0)
        } else {
            QueueSnapshot::new(SNAPSHOT_VERSION_UNINITIALIZED, 0, [0; 32])
        };
        let params = rpc_params![
            vec![source.saturating_add(1)],
            Some(anchor.block),
            Some(anchor_hash)
        ];
        let response: MmrLeavesProof = self
            .api
            .api
            .rpc()
            .request("mmr_generateProof", params)
            .await
            .context("request source MMR proof")?;
        ensure!(
            parse_h256(&response.block_hash)? == anchor.block_hash,
            "MMR proof anchor differs from requested block"
        );
        let leaves = decode_hex(&response.leaves)?;
        let mut leaves_input = &leaves[..];
        let encoded_leaves: Vec<Vec<u8>> =
            Vec::decode(&mut leaves_input).context("decode MMR leaves")?;
        ensure!(leaves_input.is_empty(), "MMR leaves have trailing bytes");
        ensure!(
            encoded_leaves.len() == 1,
            "MMR proof must contain one source leaf"
        );
        let raw_leaf = encoded_leaves.into_iter().next().expect("checked one leaf");
        let leaf = decode_outer_leaf(&raw_leaf)?;
        let proof_bytes = decode_hex(&response.proof)?;
        let mut proof_input = &proof_bytes[..];
        let proof =
            RuntimeLeafProof::decode(&mut proof_input).context("decode source MMR proof")?;
        ensure!(proof_input.is_empty(), "MMR proof has trailing bytes");
        ensure!(
            proof.leaf_indices == vec![u64::from(source)],
            "MMR proof leaf index is not the source insertion"
        );
        ensure!(
            leaf.parent_number_and_hash.0 == source,
            "MMR leaf parent number does not match source block"
        );
        ensure!(
            leaf.parent_number_and_hash.1 == H256(source_hash).0,
            "MMR leaf parent hash does not match source block"
        );
        let (_, insertion_next) = self.checkpoint_at(source.saturating_add(1)).await?;
        ensure!(
            leaf.beefy_next_authority_set.id == insertion_next.id,
            "MMR leaf next authority id differs from insertion checkpoint"
        );
        ensure!(
            leaf.beefy_next_authority_set.len == insertion_next.keys.len() as u32,
            "MMR leaf next authority length differs from insertion checkpoint"
        );
        ensure!(
            leaf.beefy_next_authority_set.keyset_commitment == insertion_next.root,
            "MMR leaf next authority root differs from insertion checkpoint"
        );
        let count = mmr_leaf_count(&self.api, anchor_hash).await?;
        ensure!(
            count == proof.leaf_count,
            "MMR API leaf count differs from proof"
        );
        ensure!(
            count == u64::from(anchor.block),
            "fresh-chain MMR start is not block one"
        );
        validate_mmr_coordinates(&proof, 1, source, u64::from(anchor.block))?;
        let root = mmr_root(&self.api, anchor_hash).await?;
        ensure!(
            root == anchor.validated.mmr_root,
            "MMR API root differs from signed BEEFY root"
        );
        let leaf_hash = outer_leaf_hash(&leaf)?;
        ensure!(
            verify_native_mmr_proof(root, &proof, leaf_hash)?,
            "native MMR proof validation failed"
        );
        let simplified = convert_mmr_proof(&proof)?;
        Ok(SourceProof {
            source,
            source_hash,
            snapshot,
            leaf,
            raw_leaf,
            proof,
            simplified,
        })
    }

    pub async fn rotate(&self, authority: &str, uri: &str) -> Result<RotationEvidence> {
        rotate(&self.api, authority, uri).await
    }
}

async fn checkpoint_at(api: &GearApi, block: u32) -> Result<(AuthoritySet, AuthoritySet)> {
    let hash = api.block_number_to_hash(block).await?;
    let validator_bytes = api
        .api
        .legacy()
        .state_call("BeefyApi_validator_set", None, Some(hash))
        .await?;
    let validator_set: Option<ValidatorSet<AuthorityId>> =
        decode_exact(&validator_bytes).context("decode historical BEEFY validator set")?;
    let validator_set = validator_set
        .ok_or_else(|| anyhow!("historical BEEFY validator set is empty at block {block}"))?;
    let current_proof = api
        .api
        .legacy()
        .state_call("BeefyMmrApi_authority_set_proof", None, Some(hash))
        .await?;
    let current_proof: BeefyAuthoritySet<Hash32> =
        decode_exact(&current_proof).context("decode historical current authority proof")?;
    let next_proof = api
        .api
        .legacy()
        .state_call("BeefyMmrApi_next_authority_set_proof", None, Some(hash))
        .await?;
    let next_proof: BeefyAuthoritySet<Hash32> =
        decode_exact(&next_proof).context("decode historical next authority proof")?;
    let current_keys: Vec<Vec<u8>> = validator_set
        .validators()
        .iter()
        .map(|key| <AuthorityId as AsRef<[u8]>>::as_ref(key).to_vec())
        .collect();
    ensure!(
        validator_set.id() == current_proof.id,
        "historical validator set id differs from proof"
    );
    ensure!(
        current_keys.len() == current_proof.len as usize,
        "historical validator set length differs from proof"
    );
    let current = AuthoritySet {
        id: current_proof.id,
        root: current_proof.keyset_commitment,
        keys: current_keys,
    };
    let next_keys: Vec<AuthorityId> = storage_decode(api, "Beefy", "NextAuthorities", hash, vec![])
        .await?
        .ok_or_else(|| {
            anyhow!("historical next BEEFY authority list is unavailable at block {block}")
        })?;
    let next = AuthoritySet {
        id: next_proof.id,
        root: next_proof.keyset_commitment,
        keys: next_keys
            .iter()
            .map(|key| <AuthorityId as AsRef<[u8]>>::as_ref(key).to_vec())
            .collect(),
    };
    ensure!(
        next.id == current.id + 1,
        "historical BEEFY authority ids are not successive"
    );
    ensure!(
        next.keys.len() == next_proof.len as usize,
        "historical next authority length differs from proof"
    );
    ensure!(
        authority_set_root(&current.keys)? == current.root,
        "historical current authority root is inconsistent"
    );
    ensure!(
        authority_set_root(&next.keys)? == next.root,
        "historical next authority root is inconsistent"
    );
    Ok((current, next))
}

async fn rotate(api: &GearApi, authority: &str, uri: &str) -> Result<RotationEvidence> {
    let authority = match authority.to_ascii_lowercase().as_str() {
        "alice" => "Alice",
        "bob" => "Bob",
        other => bail!("unsupported local authority {other}"),
    };
    let stash_pair = sr25519::Pair::from_string(&format!("//{authority}//stash"), None)
        .with_context(|| format!("derive {authority} stash"))?;
    let stash = stash_pair.public().0;
    let block_hash = api.latest_finalized_block().await?;
    let bonded: [u8; 32] = storage_decode(
        api,
        "Staking",
        "Bonded",
        block_hash,
        vec![Value::from_bytes(stash)],
    )
    .await?
    .context("validator stash is not bonded")?;
    ensure!(
        bonded == stash,
        "Staking.Bonded does not map the signer to its validator stash"
    );
    let ledger = storage_encoded(
        api,
        "Staking",
        "Ledger",
        block_hash,
        vec![Value::from_bytes(stash)],
    )
    .await?
    .context("validator stash has no staking ledger")?;
    ensure!(
        ledger.get(..32) == Some(stash.as_slice()),
        "Staking.Ledger belongs to another stash"
    );
    let validators: Vec<[u8; 32]> =
        storage_decode(api, "Session", "Validators", block_hash, vec![])
            .await?
            .ok_or_else(|| anyhow!("Session validators storage is unavailable"))?;
    ensure!(
        validators.iter().any(|key| key == &stash),
        "{authority} stash is not an active validator"
    );
    let beefy_pair = BeefyPair::from_string(uri, None).context("derive requested BEEFY key")?;
    let beefy_key = <AuthorityId as AsRef<[u8]>>::as_ref(&beefy_pair.public()).to_vec();
    ensure!(
        beefy_key.len() == 33 && (beefy_key[0] == 2 || beefy_key[0] == 3),
        "BEEFY key is not compressed SEC1"
    );
    let inserted: JsonValue = api
        .api
        .rpc()
        .request(
            "author_insertKey",
            rpc_params![
                AUTHORITY_KEY_TYPE,
                uri,
                format!("0x{}", hex::encode(&beefy_key))
            ],
        )
        .await
        .context("insert local BEEFY key")?;
    ensure!(
        inserted.is_null() || inserted.is_string(),
        "author_insertKey returned an unexpected response"
    );
    let address = subxt::dynamic::storage("Session", "NextKeys", vec![Value::from_bytes(stash)]);
    let stored = api
        .api
        .storage()
        .at(block_hash)
        .fetch(&address)
        .await?
        .context("Session.NextKeys has no entry for active stash")?;
    let mut key_value = stored.to_value()?.remove_context();
    let gsdk::ext::scale_value::ValueDef::Composite(gsdk::ext::scale_value::Composite::Named(
        fields,
    )) = &mut key_value.value
    else {
        bail!("Session.NextKeys is not a named key bundle in live metadata");
    };
    ensure!(
        fields.len() == 5,
        "runtime does not expose the expected five session keys"
    );
    let (_, key) = fields
        .iter_mut()
        .find(|(name, _)| name == "beefy")
        .context("session bundle has no BEEFY key")?;
    *key = Value::from_bytes(&beefy_key);
    let call = gsdk::ext::subxt::tx::dynamic(
        "Session",
        "set_keys",
        vec![key_value, Value::from_bytes([])],
    );
    let (extrinsic_hash, finalized_hash) = submit(api, call, stash_pair).await?;
    let block = api.block_hash_to_number(finalized_hash).await?;
    Ok(RotationEvidence {
        authority: authority.to_owned(),
        stash,
        beefy_key,
        extrinsic_hash,
        block,
        block_hash: finalized_hash.0,
    })
}

pub async fn fund_stash(api: &GearApi) -> Result<(Hash32, H256)> {
    let alice =
        sr25519::Pair::from_string("//Alice", None).context("derive Alice funding signer")?;
    let stash = sr25519::Pair::from_string("//Alice//stash", None)
        .context("derive Alice stash")?
        .public();
    let call = gsdk::ext::subxt::tx::dynamic(
        "Balances",
        "transfer_keep_alive",
        vec![
            Value::unnamed_variant("Id", [Value::from_bytes(stash.0)]),
            Value::u128(100_000_000_000_000),
        ],
    );
    submit(api, call, alice)
        .await
        .context("fund bonded stash transaction fees")
}

pub async fn send_message(
    api: &GearApi,
    receiver: [u8; 20],
    payload: &[u8],
) -> Result<ObservedMessage> {
    let pair = sr25519::Pair::from_string("//Charlie", None).context("derive Charlie signer")?;
    let sender = pair.public().0;
    let call = gsdk::ext::subxt::tx::dynamic(
        "GearEthBridge",
        "send_eth_message",
        vec![Value::from_bytes(receiver), Value::from_bytes(payload)],
    );
    let (progress, block_hash) = submit_in_block(api, call, pair).await?;
    ensure!(
        has_event(api, block_hash, "GearEthBridge", "MessageQueued").await?,
        "included message transaction has no dynamic MessageQueued event"
    );
    let messages = api.message_queued_events(block_hash).await?;
    let message = messages
        .into_iter()
        .find(|message| {
            message.source == sender
                && message.destination == receiver
                && message.payload == payload
        })
        .ok_or_else(|| {
            anyhow!("MessageQueued event does not contain requested source/destination/payload")
        })?;
    let block = api.block_hash_to_number(block_hash).await?;
    let (queue_id, queue_root) = api.fetch_queue_merkle_root(block_hash).await?;
    let initialized = bridge_initialized(api, block_hash).await?;
    ensure!(
        initialized,
        "message was queued before bridge initialization"
    );
    let snapshot = QueueSnapshot::new(0, queue_id, queue_root.0);
    let message_hash = crate::message_hash(&message);
    let inclusion = api
        .fetch_message_inclusion_merkle_proof(block_hash, H256(message_hash))
        .await
        .context("retain message inclusion proof before queue clear")?;
    ensure!(
        inclusion.root == snapshot.queue_root,
        "message inclusion proof root differs from source queue snapshot"
    );
    ensure!(
        inclusion.num_leaves > inclusion.leaf_index,
        "message inclusion index is outside queue"
    );
    let best: H256 = api
        .api
        .rpc()
        .request("chain_getBlockHash", rpc_params![])
        .await?;
    let retained_at_block = api.block_hash_to_number(best).await?;
    ensure!(
        api.fetch_queue_merkle_root(best).await?.0 == snapshot.queue_id,
        "queue cleared before message proof was retained"
    );
    let finalized = timeout(MAX_WAIT, progress.wait_for_finalized())
        .await
        .context("timed out waiting for message finality")??;
    ensure!(
        finalized.block_hash() == block_hash,
        "message reorged after its proof was retained"
    );
    Ok(ObservedMessage {
        block,
        block_hash: block_hash.0,
        message,
        message_hash,
        inclusion,
        snapshot,
        retained_at_block,
    })
}

pub async fn initialize_bridge(api: &GearApi) -> Result<()> {
    let mut cursor = api
        .block_hash_to_number(api.latest_finalized_block().await?)
        .await?
        .saturating_sub(32);
    let deadline = Instant::now() + MAX_WAIT;
    let mut observed = false;
    loop {
        let head_hash = api.latest_finalized_block().await?;
        let head = api.block_hash_to_number(head_hash).await?;
        while cursor <= head {
            let hash = api.block_number_to_hash(cursor).await?;
            if has_event(api, hash, "GearEthBridge", "BridgeInitialized").await? {
                observed = true;
                cursor = head.saturating_add(1);
                break;
            }
            cursor = cursor.saturating_add(1);
        }
        if observed && bridge_initialized(api, head_hash).await? {
            break;
        }
        ensure!(
            Instant::now() < deadline,
            "timed out waiting for BridgeInitialized"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    if bridge_paused(api, api.latest_finalized_block().await?).await? {
        let alice =
            sr25519::Pair::from_string("//Alice", None).context("derive Alice sudo signer")?;
        let inner = gsdk::ext::subxt::tx::dynamic("GearEthBridge", "unpause", Vec::<Value>::new())
            .into_value();
        let call = gsdk::ext::subxt::tx::dynamic("Sudo", "sudo", vec![inner]);
        let (_, block_hash) = submit_in_block(api, call, alice).await?;
        ensure!(
            has_event(api, block_hash, "GearEthBridge", "BridgeUnpaused").await?,
            "unpause included without BridgeUnpaused"
        );
        ensure!(
            !bridge_paused(api, block_hash).await?,
            "bridge remains paused after sudo unpause"
        );
    }
    Ok(())
}

pub async fn validate_runtime(api: &GearApi) -> Result<JsonValue> {
    let genesis = api.block_number_to_hash(0).await?;
    let babe = api
        .api
        .legacy()
        .state_call("BabeApi_configuration", None, Some(genesis))
        .await?;
    ensure!(
        babe.len() >= 16,
        "BabeApi_configuration response is truncated"
    );
    let slot_duration = u64::from_le_bytes(babe[0..8].try_into().expect("fixed Babe slot width"));
    let epoch_length = u64::from_le_bytes(babe[8..16].try_into().expect("fixed Babe epoch width"));
    ensure!(
        slot_duration == 3_000,
        "unexpected BABE slot duration {slot_duration}"
    );
    ensure!(
        epoch_length == 8,
        "unexpected fast-runtime BABE epoch length {epoch_length}"
    );
    let code: Option<String> = api
        .api
        .rpc()
        .request("state_getStorage", rpc_params!["0x3a636f6465", genesis])
        .await?;
    let code = decode_hex(&code.context("runtime :code missing at genesis")?)?;
    let activation = api
        .api
        .legacy()
        .state_call("BeefyApi_beefy_genesis", None, Some(genesis))
        .await?;
    ensure!(
        decode_exact::<Option<u32>>(&activation)? == Some(1),
        "local BEEFY activation is not block one"
    );
    ensure!(
        mmr_leaf_count(api, genesis).await? == 0,
        "genesis already contains MMR leaves"
    );
    let (current, next) = checkpoint_at(api, 0).await?;
    Ok(json!({
        "genesisHash": format!("0x{}", hex::encode(genesis.0)),
        "runtimeCodeHash": format!("0x{}", hex::encode(keccak256(&code))),
        "babe": {"slotDuration": slot_duration, "epochLength": epoch_length},
        "authorityGenesis": {"current": current, "next": next},
    }))
}

async fn mmr_root(api: &GearApi, at: H256) -> Result<Hash32> {
    let bytes = api
        .api
        .legacy()
        .state_call("MmrApi_mmr_root", None, Some(at))
        .await?;
    decode_exact::<std::result::Result<Hash32, sp_mmr_primitives::Error>>(&bytes)?
        .map_err(|error| anyhow!("MMR root API returned {error:?}"))
}

async fn mmr_leaf_count(api: &GearApi, at: H256) -> Result<u64> {
    let bytes = api
        .api
        .legacy()
        .state_call("MmrApi_mmr_leaf_count", None, Some(at))
        .await?;
    decode_exact::<std::result::Result<u64, sp_mmr_primitives::Error>>(&bytes)?
        .map_err(|error| anyhow!("MMR leaf-count API returned {error:?}"))
}

async fn bridge_initialized(api: &GearApi, at: H256) -> Result<bool> {
    Ok(
        storage_decode(api, "GearEthBridge", "Initialized", at, vec![])
            .await?
            .unwrap_or(false),
    )
}

async fn bridge_paused(api: &GearApi, at: H256) -> Result<bool> {
    Ok(storage_decode(api, "GearEthBridge", "Paused", at, vec![])
        .await?
        .unwrap_or(true))
}

async fn has_event(api: &GearApi, at: H256, pallet: &str, variant: &str) -> Result<bool> {
    let block = api.get_block_at(at).await?;
    let events = block.events().await?;
    for event in events.iter() {
        let event = event?;
        if event.pallet_name() == pallet && event.variant_name() == variant {
            return Ok(true);
        }
    }
    Ok(false)
}

async fn storage_decode<T: Decode>(
    api: &GearApi,
    pallet: &str,
    entry: &str,
    at: H256,
    keys: Vec<Value>,
) -> Result<Option<T>> {
    let address = subxt::dynamic::storage(pallet, entry, keys);
    let Some(value) = api.api.storage().at(at).fetch(&address).await? else {
        return Ok(None);
    };
    let mut input = value.encoded();
    let decoded =
        T::decode(&mut input).with_context(|| format!("decode {pallet}.{entry} storage"))?;
    ensure!(
        input.is_empty(),
        "{pallet}.{entry} storage has trailing bytes"
    );
    Ok(Some(decoded))
}

async fn storage_encoded(
    api: &GearApi,
    pallet: &str,
    entry: &str,
    at: H256,
    keys: Vec<Value>,
) -> Result<Option<Vec<u8>>> {
    let address = subxt::dynamic::storage(pallet, entry, keys);
    Ok(api
        .api
        .storage()
        .at(at)
        .fetch(&address)
        .await?
        .map(|value| value.into_encoded()))
}

async fn submit_watched(
    api: &GearApi,
    call: gsdk::ext::subxt::tx::DynamicPayload,
    pair: sr25519::Pair,
) -> Result<subxt::tx::TxProgress<gsdk::GearConfig, subxt::OnlineClient<gsdk::GearConfig>>> {
    let signer = PairSigner::<gsdk::GearConfig, sr25519::Pair>::new(pair);
    api.api
        .tx()
        .sign_and_submit_then_watch_default(&call, &signer)
        .await
        .map_err(|error| anyhow!("submit dynamic extrinsic: {error:?}"))
}

async fn submit_in_block(
    api: &GearApi,
    call: gsdk::ext::subxt::tx::DynamicPayload,
    pair: sr25519::Pair,
) -> Result<(
    subxt::tx::TxProgress<gsdk::GearConfig, subxt::OnlineClient<gsdk::GearConfig>>,
    H256,
)> {
    use subxt::tx::TxStatus;
    let mut progress = submit_watched(api, call, pair).await?;
    let block_hash = timeout(MAX_WAIT, async {
        loop {
            match progress
                .next()
                .await
                .context("transaction status subscription closed")??
            {
                TxStatus::InBestBlock(block) => {
                    block
                        .wait_for_success()
                        .await
                        .context("included extrinsic failed")?;
                    break Ok::<_, anyhow::Error>(block.block_hash());
                }
                TxStatus::Error { message }
                | TxStatus::Invalid { message }
                | TxStatus::Dropped { message } => bail!("transaction rejected: {message}"),
                TxStatus::InFinalizedBlock(_) => {
                    bail!("transaction finalized before inclusion could be captured")
                }
                TxStatus::NoLongerInBestBlock => {
                    bail!("transaction reorged before inclusion capture")
                }
                _ => {}
            }
        }
    })
    .await
    .context("timed out waiting for transaction inclusion")??;
    Ok((progress, block_hash))
}

async fn submit(
    api: &GearApi,
    call: gsdk::ext::subxt::tx::DynamicPayload,
    pair: sr25519::Pair,
) -> Result<(Hash32, H256)> {
    let progress = submit_watched(api, call, pair).await?;
    let tx_hash = progress.extrinsic_hash().0;
    let finalized = timeout(MAX_WAIT, progress.wait_for_finalized())
        .await
        .map_err(|_| anyhow!("timed out waiting for extrinsic finality"))??;
    let block_hash = finalized.block_hash();
    finalized
        .wait_for_success()
        .await
        .context("finalized extrinsic failed")?;
    Ok((tx_hash, block_hash))
}

fn authority_set_root(keys: &[Vec<u8>]) -> Result<Hash32> {
    let keys = key_arrays(keys)?;
    authority_root(&authority_addresses(&keys)?)
}

fn key_arrays(keys: &[Vec<u8>]) -> Result<Vec<[u8; 33]>> {
    keys.iter()
        .map(|key| {
            key.as_slice()
                .try_into()
                .map_err(|_| anyhow!("authority key is not 33 bytes"))
        })
        .collect()
}

fn decode_exact<T: Decode>(bytes: &[u8]) -> Result<T> {
    let mut input = bytes;
    let value = T::decode(&mut input)?;
    ensure!(input.is_empty(), "SCALE value has trailing bytes");
    Ok(value)
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    hex::decode(value.strip_prefix("0x").unwrap_or(value)).context("decode RPC hex")
}

fn parse_h256(value: &str) -> Result<Hash32> {
    let bytes = decode_hex(value)?;
    ensure!(bytes.len() == 32, "RPC hash is not 32 bytes");
    Ok(bytes.try_into().expect("checked hash length"))
}

fn justification_bytes(value: &JsonValue) -> Result<Option<Vec<u8>>> {
    let justifications = value
        .get("justifications")
        .context("chain_getBlock omitted justifications")?;
    if justifications.is_null() {
        return Ok(None);
    }
    let entries = justifications
        .as_array()
        .context("invalid block justifications")?;
    let mut found = None;
    for entry in entries {
        let pair = entry.as_array().context("invalid justification pair")?;
        ensure!(pair.len() == 2, "invalid justification pair length");
        let engine: [u8; 4] =
            serde_json::from_value(pair[0].clone()).context("invalid consensus engine id")?;
        if engine == BEEFY_ENGINE {
            ensure!(found.is_none(), "duplicate BEEFY engine justification");
            found = Some(
                serde_json::from_value(pair[1].clone())
                    .context("invalid BEEFY justification byte array")?,
            );
        }
    }
    Ok(found)
}
