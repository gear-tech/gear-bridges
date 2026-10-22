use anyhow::{anyhow, ensure, Context, Result};
#[cfg(test)]
use gear_rpc_client::dto;
use gear_rpc_client::GearApi;
use gsdk::{ext::subxt::utils::H256, Value};
#[cfg(any(test, feature = "deployment"))]
use parity_scale_codec::Compact;
use parity_scale_codec::{Decode, Encode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as JsonValue};
use sp_consensus_beefy::{ecdsa_crypto::AuthorityId, mmr::BeefyAuthoritySet};
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
};
use subxt_rpcs::{client::RpcSubscription, rpc_params};
#[cfg(any(test, feature = "deployment"))]
use tokio::time::Instant;
use tokio::time::{timeout, Duration};

use crate::{
    authority_addresses, authority_root, decode_authority_list,
    decode_beefy_digest, decode_mmr_leaves, decode_mmr_proof, decode_outer_leaf,
    decode_validator_set, decode_versioned_finality_proof, keccak256, outer_leaf_hash,
    validate_mmr_coordinates, validate_signed_commitment, verify_native_mmr_proof, Hash32,
    QueueSnapshot, RuntimeLeaf, RuntimeLeafProof, SimplifiedMmrProof, ValidatedCommitment,
    MAX_BEEFY_PROOF_BYTES, MAX_MMR_LEAVES_BYTES, MAX_MMR_PROOF_BYTES, MAX_VALIDATORS,
};

const MAX_WAIT: Duration = Duration::from_secs(45);
const MAX_ACCEPTED_IDENTITIES: usize = 4096;
const MAX_PENDING_JUSTIFICATIONS: usize = 1024;

#[derive(Debug)]
pub struct CommitmentPending;
impl std::fmt::Display for CommitmentPending {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(
            "complete ordered source commitment is pending; retain durable scan progress",
        )
    }
}
impl std::error::Error for CommitmentPending {}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScanJournal {
    genesis: Hash32,
    bridge_domain: Hash32,
    scanned_through: u32,
    scanned_hash: Hash32,
    pending: std::collections::BTreeMap<u32, Vec<u8>>,
    delivered: Option<(u32, Vec<u8>)>,
}

impl ScanJournal {
    fn rewind_to(&mut self, accepted: u32, hash: Hash32) {
        // GRANDPA finality does not freeze BEEFY justifications: missing proofs
        // may have been appended behind the saved scan cursor while offline.
        self.scanned_through = accepted;
        self.scanned_hash = hash;
    }
}

fn commitment_identity(raw: &[u8]) -> Result<Hash32> {
    Ok(keccak256(
        &decode_versioned_finality_proof(raw)?.commitment.encode(),
    ))
}

const BEEFY_ENGINE: [u8; 4] = *b"BEEF";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthoritySet {
    pub id: u64,
    pub keys: Vec<Vec<u8>>,
    pub root: Hash32,
}

#[derive(Clone, Debug)]
pub struct CapturedCommitment {
    pub block: u32,
    pub block_hash: Hash32,
    pub finalized_height: u32,
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
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MmrLeavesProof {
    block_hash: String,
    leaves: String,
    proof: String,
}

pub struct Source {
    pub api: GearApi,
    pub source_genesis: Hash32,
    pub bridge_domain: Hash32,
    pub mmr_start_block: u64,
    pub beefy_activation_block: u64,
    pub identity: SourceIdentity,
    subscription: RpcSubscription<String>,
    pending: std::collections::BTreeMap<u32, Vec<u8>>,
    accepted: std::collections::BTreeMap<u32, Hash32>,
    last_block: Option<u32>,
    last_set_id: Option<u64>,
    scanned_through: u32,
    scanned_hash: Hash32,
    scan_path: Option<PathBuf>,
    delivered: Option<(u32, Vec<u8>)>,
}

impl Source {
    #[cfg(test)]
    pub async fn connect(api: GearApi) -> Result<Self> {
        let subscription = Self::subscribe(&api).await?;
        let at = api.latest_finalized_block().await?;
        let identity = discover_source_identity(&api, at).await?;
        Ok(Self::from_identity(api, subscription, identity))
    }

    /// Subscribe on both nodes before discovery or catch-up; compare one common
    /// finalized upgraded pin, never unrelated node heads or genesis APIs.
    pub async fn connect_pair(api: GearApi, witness: GearApi) -> Result<(Self, Self)> {
        let (subscription, witness_subscription) =
            tokio::try_join!(Self::subscribe(&api), Self::subscribe(&witness))?;
        let (peer, witness_peer): (String, String) = tokio::try_join!(
            api.api.rpc().request("system_localPeerId", rpc_params![]),
            witness
                .api
                .rpc()
                .request("system_localPeerId", rpc_params![])
        )?;
        ensure!(
            !peer.is_empty() && !witness_peer.is_empty() && peer != witness_peer,
            "source and independent witness must be different nodes; HOLD"
        );
        let (head, witness_head) = tokio::try_join!(
            api.latest_finalized_block(),
            witness.latest_finalized_block()
        )?;
        let (height, witness_height) = tokio::try_join!(
            api.block_hash_to_number(head),
            witness.block_hash_to_number(witness_head)
        )?;
        let common = height.min(witness_height);
        let (at, witness_at) = tokio::try_join!(
            api.block_number_to_hash(common),
            witness.block_number_to_hash(common)
        )?;
        ensure!(
            at == witness_at,
            "source/witness common finalized hash differs; HOLD"
        );
        let (identity, witness_identity) = tokio::try_join!(
            discover_source_identity(&api, at),
            discover_source_identity(&witness, witness_at)
        )?;
        ensure_paired_identity(&identity, &witness_identity)?;
        Ok((
            Self::from_identity(api, subscription, identity),
            Self::from_identity(witness, witness_subscription, witness_identity),
        ))
    }

    pub async fn validate_attachment(&self, anchor: &JsonValue) -> Result<()> {
        if anchor["sourceIdentity"].is_null() {
            // Retained lanes have immutable old anchors. They are the explicit
            // genesis/fast profile, never an implicit normal-runtime attachment.
            let legacy = validate_legacy_runtime(&self.api).await?;
            ensure_runtime_binding(&legacy, &self.identity.runtime)?;
        } else {
            let pinned = &anchor["sourceIdentity"];
            ensure_runtime_binding(&pinned["runtime"], &self.identity.runtime)?;
            ensure!(pinned["beefyActivationBlock"] == self.beefy_activation_block
                && pinned["mmrStartBlock"] == self.mmr_start_block
                && pinned["domainBindingBlock"] == self.identity.domain_binding_block,
                "source activation, MMR start or domain binding changed from immutable anchor; HOLD");
        }
        Ok(())
    }

    async fn subscribe(api: &GearApi) -> Result<RpcSubscription<String>> {
        api.api
            .rpc()
            .subscribe(
                "beefy_subscribeJustifications",
                rpc_params![],
                "beefy_unsubscribeJustifications",
            )
            .await
            .context("subscribe to BEEFY justifications")
    }

    fn from_identity(
        api: GearApi,
        subscription: RpcSubscription<String>,
        identity: SourceIdentity,
    ) -> Self {
        Self {
            api,
            source_genesis: identity.source_genesis,
            bridge_domain: identity.bridge_domain,
            mmr_start_block: identity.mmr_start_block,
            beefy_activation_block: identity.beefy_activation_block,
            identity,
            subscription,
            pending: Default::default(),
            accepted: Default::default(),
            last_block: None,
            last_set_id: None,
            scanned_through: 0,
            scanned_hash: [0; 32],
            scan_path: None,
            delivered: None,
        }
    }

    /// Start a new client at the current finalized head without replaying old handovers.
    pub async fn seek_to_finalized(&mut self) -> Result<u32> {
        ensure!(
            self.last_block.is_none() && self.pending.is_empty(),
            "source cursor already used"
        );
        let finalized = self
            .api
            .block_hash_to_number(self.api.latest_finalized_block().await?)
            .await?;
        self.scanned_through = finalized.saturating_sub(1);
        Ok(finalized)
    }
    /// Resume at the saved client checkpoint so historical handovers are not skipped.
    pub fn seek_from(&mut self, block: u32) -> Result<()> {
        ensure!(
            block > 0 && self.last_block.is_none() && self.pending.is_empty(),
            "source cursor already used or invalid resume block"
        );
        self.scanned_through = block - 1;
        Ok(())
    }

    /// The cursor and every undispatched original justification commit together.
    /// A returned-but-not-journaled handover is requeued on restart.
    /// Rescan after the accepted BEEFY checkpoint: finalized blocks may gain proofs later.
    pub async fn enable_scan_journal(
        &mut self,
        path: &Path,
        resume_from: u32,
        witness: &Source,
    ) -> Result<()> {
        ensure!(
            self.scan_path.is_none() && self.last_block.is_none(),
            "source scan journal already configured"
        );
        if path.exists() {
            let mut saved: ScanJournal = serde_json::from_slice(&fs::read(path)?)
                .context("invalid source scan journal; HOLD")?;
            ensure!(
                saved.genesis == self.source_genesis
                    && saved.bridge_domain == self.bridge_domain
                    && witness.source_genesis == self.source_genesis
                    && witness.bridge_domain == self.bridge_domain,
                "source scan journal belongs to another witnessed lane; HOLD"
            );
            let finalized = self
                .api
                .block_hash_to_number(self.api.latest_finalized_block().await?)
                .await?;
            let second_finalized = witness
                .api
                .block_hash_to_number(witness.api.latest_finalized_block().await?)
                .await?;
            ensure!(
                saved.scanned_through <= finalized && saved.scanned_through <= second_finalized,
                "saved source cursor is ahead of finalized witnesses; HOLD"
            );
            let (first, second) = tokio::try_join!(
                self.api.block_number_to_hash(saved.scanned_through),
                witness.api.block_number_to_hash(saved.scanned_through)
            )?;
            ensure!(
                first.0 == saved.scanned_hash && second == first,
                "source cursor changed canonical history; HOLD"
            );
            ensure!(
                saved.pending.len() <= MAX_PENDING_JUSTIFICATIONS,
                "oversized pending source scan journal; HOLD"
            );
            saved.rewind_to(
                resume_from,
                self.api.block_number_to_hash(resume_from).await?.0,
            );
            self.scanned_through = saved.scanned_through;
            self.scanned_hash = saved.scanned_hash;
            for (block, raw) in saved.pending.into_iter().chain(saved.delivered.into_iter()) {
                ensure!(
                    decode_versioned_finality_proof(&raw)?
                        .commitment
                        .block_number
                        == block,
                    "source cursor justification identity changed; HOLD"
                );
                if block > resume_from {
                    self.queue_justification(raw).await?;
                }
            }
            let (current, _) = self.checkpoint_at(resume_from).await?;
            self.last_block = Some(resume_from);
            self.last_set_id = Some(current.id);
        }
        if self.scanned_hash == [0; 32] {
            self.scanned_hash = self.api.block_number_to_hash(self.scanned_through).await?.0;
        }
        self.scan_path = Some(path.to_owned());
        self.persist_scan_journal(None)
    }

    fn persist_scan_journal(&self, delivery: Option<(u32, &[u8])>) -> Result<()> {
        let Some(path) = &self.scan_path else {
            return Ok(());
        };
        let directory = path.parent().context("source scan path has no parent")?;
        fs::create_dir_all(directory)?;
        let temporary = path.with_extension("json.tmp");
        let mut file = std::io::BufWriter::new(File::create(&temporary)?);
        // Pending raw proofs are bounded; accepted raw proofs live only in the
        // caller's immutable submission journal, never an expanding RAM archive.
        struct Undispatched<'a> {
            pending: &'a std::collections::BTreeMap<u32, Vec<u8>>,
            exclude: Option<u32>,
        }
        impl Serialize for Undispatched<'_> {
            fn serialize<S: serde::Serializer>(
                &self,
                serializer: S,
            ) -> std::result::Result<S::Ok, S::Error> {
                use serde::ser::SerializeMap;
                let count = self.pending.len()
                    - usize::from(
                        self.exclude
                            .is_some_and(|block| self.pending.contains_key(&block)),
                    );
                let mut map = serializer.serialize_map(Some(count))?;
                for (block, raw) in self.pending {
                    if self.exclude != Some(*block) {
                        map.serialize_entry(block, raw)?;
                    }
                }
                map.end()
            }
        }
        #[derive(Serialize)]
        struct Saved<'a> {
            genesis: Hash32,
            bridge_domain: Hash32,
            scanned_through: u32,
            scanned_hash: Hash32,
            pending: Undispatched<'a>,
            delivered: Option<(u32, &'a [u8])>,
        }
        serde_json::to_writer(
            &mut file,
            &Saved {
                genesis: self.source_genesis,
                bridge_domain: self.bridge_domain,
                scanned_through: self.scanned_through,
                scanned_hash: self.scanned_hash,
                pending: Undispatched {
                    pending: &self.pending,
                    exclude: delivery.map(|(block, _)| block),
                },
                delivered: delivery.or_else(|| {
                    self.delivered
                        .as_ref()
                        .map(|(block, raw)| (*block, raw.as_slice()))
                }),
            },
        )?;
        file.flush()?;
        file.get_ref().sync_all()?;
        fs::rename(temporary, path)?;
        File::open(directory)?.sync_all()?;
        Ok(())
    }

    /// Replay archived history before draining later durable originals, so late
    /// justifications cannot hide an authority transition behind the scan cursor.
    pub async fn next_commitment(&mut self) -> Result<CapturedCommitment> {
        timeout(MAX_WAIT, self.next_commitment_inner())
            .await
            .map_err(|_| anyhow!(CommitmentPending))?
    }

    async fn next_commitment_inner(&mut self) -> Result<CapturedCommitment> {
        loop {
            let finalized_hash = self.api.latest_finalized_block().await?;
            let finalized = self.api.block_hash_to_number(finalized_hash).await?;

            let next = self
                .pending
                .range(..=finalized.min(self.scanned_through))
                .next()
                .map(|(&block, raw)| (block, raw.clone()));
            let next = if next.is_some() {
                next
            } else if self.scanned_through < finalized {
                // New archive proofs go directly to durable delivery, not into a
                // possibly saturated retained backlog of original justifications.
                let Some(next) = self.scan_range(finalized).await? else {
                    continue;
                };
                Some(next)
            } else {
                None
            };
            if let Some((block, raw)) = next {
                let captured = self.capture(block, raw.clone(), finalized, true).await?;
                // Commit the original delivered proof before advancing memory.
                // An I/O failure leaves it pending and cannot lose a handover.
                self.persist_scan_journal(Some((block, &raw)))?;
                self.pending.remove(&block);
                self.accepted
                    .insert(block, captured.validated.commitment_hash);
                while self.accepted.len() > MAX_ACCEPTED_IDENTITIES {
                    self.accepted.pop_first();
                }
                self.delivered = Some((block, raw));
                self.last_block = Some(block);
                self.last_set_id = Some(captured.validated.signed.commitment.validator_set_id);
                return Ok(captured);
            }

            match timeout(Duration::from_secs(3), self.subscription.next()).await {
                Err(_) => {}
                Ok(None) => {
                    return Err(
                        subxt::Error::Rpc(subxt::error::RpcError::SubscriptionDropped).into(),
                    )
                }
                Ok(Some(Err(error))) => {
                    return Err(anyhow!(error).context("BEEFY justification subscription failed"))
                }
                Ok(Some(Ok(raw))) => {
                    self.queue_justification(decode_hex_bounded(&raw, MAX_BEEFY_PROOF_BYTES)?)
                        .await?;
                }
            }
        }
    }

    async fn scan_range(&mut self, finalized: u32) -> Result<Option<(u32, Vec<u8>)>> {
        let Some(start) = self.scanned_through.checked_add(1) else {
            return Ok(None);
        };
        if start > finalized {
            return Ok(None);
        }
        let end = finalized.min(start.saturating_add(63));
        for block in start..=end {
            let hash = self.api.block_number_to_hash(block).await?;
            let raw = self
                .api
                .api
                .rpc()
                .request::<JsonValue>("chain_getBlock", rpc_params![hash])
                .await
                .with_context(|| format!("read block {block} for BEEFY catch-up"))?;
            let bytes = justification_bytes(&raw)?;
            self.scanned_through = block;
            self.scanned_hash = hash.0;
            if let Some(bytes) = bytes {
                let proof_block = decode_versioned_finality_proof(&bytes)?
                    .commitment
                    .block_number;
                ensure!(proof_block > 0, "genesis BEEFY commitment is not usable");
                if self.last_block.is_none_or(|last| proof_block > last)
                    && !self.pending.contains_key(&proof_block)
                    && proof_block <= block
                {
                    return Ok(Some((proof_block, bytes)));
                }
                self.queue_justification(bytes).await?;
            }
            if self.pending.range(..=block).next().is_some() {
                self.persist_scan_journal(None)?;
                return Ok(None);
            }
            if block % 32 == 0 {
                self.persist_scan_journal(None)?;
            }
        }
        self.persist_scan_journal(None)?;
        Ok(None)
    }
    async fn queue_justification(&mut self, bytes: Vec<u8>) -> Result<()> {
        let signed =
            decode_versioned_finality_proof(&bytes).context("decode BEEFY justification")?;
        let block = signed.commitment.block_number;
        ensure!(block > 0, "genesis BEEFY commitment is not usable");
        let identity = keccak256(&signed.commitment.encode());
        if let Some(existing) = self.accepted.get(&block) {
            ensure!(
                *existing == identity,
                "conflicting BEEFY commitment for accepted block {block}"
            );
            self.recapture(block, bytes).await?;
            return Ok(());
        }
        if let Some(previous) = self.last_block {
            if block <= previous {
                // Older semantic identities may leave the bounded hot cache.
                // Reauthenticate against the original archived justification.
                let hash = self.api.block_number_to_hash(block).await?;
                let original: JsonValue = self
                    .api
                    .api
                    .rpc()
                    .request("chain_getBlock", rpc_params![hash])
                    .await?;
                let original = justification_bytes(&original)?
                    .context("original accepted BEEFY justification unavailable; HOLD")?;
                ensure!(
                    commitment_identity(&original)? == identity,
                    "conflicting historical BEEFY commitment at block {block}"
                );
                self.recapture(block, bytes).await?;
                return Ok(());
            }
        }
        if let Some(existing) = self.pending.get(&block) {
            ensure!(
                commitment_identity(existing)? == identity,
                "conflicting BEEFY commitment for block {block}"
            );
            if existing != &bytes {
                self.recapture(block, bytes).await?;
            }
        } else {
            ensure!(
                self.pending.len() < MAX_PENDING_JUSTIFICATIONS,
                "pending source justifications reached the bounded limit; HOLD"
            );
            self.pending.insert(block, bytes);
        }
        Ok(())
    }

    pub async fn recapture(&self, block: u32, raw: Vec<u8>) -> Result<CapturedCommitment> {
        let finalized = self
            .api
            .block_hash_to_number(self.api.latest_finalized_block().await?)
            .await?;
        if block > finalized {
            return Err(anyhow!(CommitmentPending));
        }
        self.recapture_at_finalized(block, raw, finalized).await
    }

    pub(crate) async fn recapture_at_finalized(
        &self,
        block: u32,
        raw: Vec<u8>,
        finalized: u32,
    ) -> Result<CapturedCommitment> {
        self.capture(block, raw, finalized, false).await
    }

    async fn capture(
        &self,
        block: u32,
        raw: Vec<u8>,
        finalized_height: u32,
        check_history: bool,
    ) -> Result<CapturedCommitment> {
        ensure!(
            block <= finalized_height,
            "BEEFY commitment at {block} is not finalized at {finalized_height}"
        );
        if check_history {
            if let Some(previous) = self.last_block {
                ensure!(
                    block > previous,
                    "BEEFY commitment history is not increasing"
                );
            }
        }
        let (current, next) = self.checkpoint_at(block).await?;
        let current_keys = key_arrays(&current.keys)?;
        let validated =
            validate_signed_commitment(&raw, &current_keys, Some(current.id), Some(block))
                .with_context(|| format!("validate signed BEEFY commitment at block {block}"))?;
        let block_hash = self.api.block_number_to_hash(block).await?;
        ensure!(
            self.mmr_digest(block).await? == Some(validated.mmr_root),
            "BEEFY commitment MMR root does not match the block digest"
        );
        let count = mmr_leaf_count(&self.api, block_hash).await?;
        ensure!(
            count == expected_leaf_count(self.mmr_start_block, block)?,
            "MMR leaf count does not match source start block at commitment {block}"
        );
        if check_history {
            if let Some(last_id) = self.last_set_id {
                ensure!(
                    validated.signed.commitment.validator_set_id == last_id
                        || validated.signed.commitment.validator_set_id
                            == last_id
                                .checked_add(1)
                                .context("BEEFY authority set id overflow")?,
                    "unknown or skipped BEEFY authority set {} after {}",
                    validated.signed.commitment.validator_set_id,
                    last_id
                );
            }
        }
        ensure!(
            validated.signed.commitment.validator_set_id == current.id,
            "commitment set id does not match historical current authority set"
        );
        Ok(CapturedCommitment {
            block,
            block_hash: block_hash.0,
            finalized_height,
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
            let bytes = decode_hex_bounded(hex_log, MAX_VALIDATORS * 33 + 32)?;
            if let Some(root) = decode_beefy_digest(&bytes)? {
                return Ok(Some(root));
            }
        }
        Ok(None)
    }

    pub async fn checkpoint_at(&self, block: u32) -> Result<(AuthoritySet, AuthoritySet)> {
        checkpoint_at(&self.api, block).await
    }

    pub async fn checkpoint_at_hash(
        &self,
        block_hash: H256,
    ) -> Result<(AuthoritySet, AuthoritySet)> {
        let block = self.api.block_hash_to_number(block_hash).await?;
        checkpoint_at_hash(&self.api, block_hash, block).await
    }

    /// Verifies leaf and authority inclusion; queue envelopes also bind the parent snapshot.
    /// Session hooks can change bridge state before a handover leaf is inserted.
    pub async fn proof(&self, source: u32, anchor: &CapturedCommitment) -> Result<SourceProof> {
        validate_application_block(
            self.mmr_start_block,
            self.identity.domain_binding_block,
            source,
            anchor.block,
        )?;
        let insertion_block = source.checked_add(1).context("source insertion overflow")?;
        let source_hash = self.api.block_number_to_hash(source).await?.0;
        let anchor_hash = H256(anchor.block_hash);
        let source_timestamp_ms = self.api.fetch_timestamp(H256(source_hash)).await?;
        let snapshot = if bridge_initialized(&self.api, H256(source_hash)).await? {
            let (queue_id, queue_root) =
                self.api.fetch_queue_merkle_root(H256(source_hash)).await?;
            QueueSnapshot::new(
                self.bridge_domain,
                source_timestamp_ms,
                true,
                queue_id,
                queue_root.0,
            )?
        } else {
            let insertion_hash = self.api.block_number_to_hash(insertion_block).await?;
            if has_event(
                &self.api,
                insertion_hash,
                "GearEthBridge",
                "BridgeInitialized",
            )
            .await?
            {
                ensure!(
                    bridge_initialized(&self.api, insertion_hash).await?,
                    "BridgeInitialized event did not leave initialized state at insertion block"
                );
                let (queue_id, _) = self.api.fetch_queue_merkle_root(insertion_hash).await?;
                QueueSnapshot::new(
                    self.bridge_domain,
                    source_timestamp_ms,
                    true,
                    queue_id,
                    [0; 32],
                )?
            } else {
                QueueSnapshot::new(self.bridge_domain, source_timestamp_ms, false, 0, [0; 32])?
            }
        };
        let params = rpc_params![vec![insertion_block], Some(anchor.block), Some(anchor_hash)];
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
        let leaves = decode_hex_bounded(&response.leaves, MAX_MMR_LEAVES_BYTES)?;
        let raw_leaf = decode_mmr_leaves(&leaves)?;
        let leaf = decode_outer_leaf(&raw_leaf)?;
        ensure!(
            leaf.leaf_extra == snapshot.hash(),
            "MMR leaf metadata hash mismatch for B={source}, L={insertion_block}, C={}",
            anchor.block
        );
        let proof_bytes = decode_hex_bounded(&response.proof, MAX_MMR_PROOF_BYTES)?;
        let proof = decode_mmr_proof(&proof_bytes)?;
        let expected_index = u64::from(insertion_block)
            .checked_sub(self.mmr_start_block)
            .context("MMR source index underflow")?;
        ensure!(
            proof.leaf_indices == [expected_index],
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
        let (_, insertion_next) = self.checkpoint_at(insertion_block).await?;
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
            count == expected_leaf_count(self.mmr_start_block, anchor.block)?,
            "MMR API leaf count does not match source start and anchor"
        );
        validate_mmr_coordinates(
            &proof,
            self.mmr_start_block,
            source,
            u64::from(anchor.block),
        )?;
        let root = mmr_root(&self.api, anchor_hash).await?;
        ensure!(
            root == anchor.validated.mmr_root,
            "MMR API root differs from signed BEEFY root"
        );
        let leaf_hash = outer_leaf_hash(&leaf)?;
        let simplified = verify_native_mmr_proof(root, &proof, leaf_hash)
            .context("native MMR proof validation failed")?;
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
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceIdentity {
    pub source_genesis: Hash32,
    pub finalized_height: u32,
    pub finalized_hash: Hash32,
    pub runtime: JsonValue,
    pub current: AuthoritySet,
    pub next: AuthoritySet,
    pub beefy_activation_block: u64,
    pub mmr_start_block: u64,
    pub bridge_domain: Hash32,
    pub domain_binding_block: u32,
    pub mmr_leaf_count: u64,
    pub mmr_root: Hash32,
    pub first_insertion_hash: Hash32,
    pub first_leaf_hash: Hash32,
}

#[derive(Debug)]
pub struct SourceActivationPending;
impl std::fmt::Display for SourceActivationPending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("source activation or first finalized MMR insertion is pending; HOLD")
    }
}
impl std::error::Error for SourceActivationPending {}

fn active_mmr_start(activation: Option<u32>, block: u32, count: u64) -> Result<(u64, u64)> {
    let activation = activation
        .filter(|g| *g > 0 && *g <= block)
        .ok_or(SourceActivationPending)?;
    if count == 0 {
        return Err(SourceActivationPending.into());
    }
    let start = u64::from(block)
        .checked_add(1)
        .and_then(|n| n.checked_sub(count))
        .filter(|s| *s > 0)
        .context("MMR count exceeds finalized source range")?;
    // MMR begins when this runtime is installed, possibly before BEEFY activation.
    ensure!(
        count == expected_leaf_count(start, block)?,
        "noncontiguous MMR range"
    );
    Ok((u64::from(activation), start))
}

fn ensure_runtime_binding(expected: &JsonValue, observed: &JsonValue) -> Result<()> {
    for name in [
        "runtimeCodeSha256",
        "runtimeCodeKeccak256",
        "runtimeCodeBlake2b256",
    ] {
        let expected = parse_h256(
            expected[name]
                .as_str()
                .context("immutable source runtime hash/algorithm is missing; HOLD")?,
        )?;
        let observed = parse_h256(
            observed[name]
                .as_str()
                .context("observed source runtime hash/algorithm is missing; HOLD")?,
        )?;
        ensure!(
            expected == observed,
            "source runtime {name} differs from immutable anchor; HOLD"
        );
    }
    Ok(())
}

fn ensure_paired_identity(source: &SourceIdentity, witness: &SourceIdentity) -> Result<()> {
    ensure!(source == witness, "source/witness runtime, authorities, activation, MMR or domain differs at common finalized pin; HOLD");
    Ok(())
}

pub(crate) async fn bridge_domain_at(api: &GearApi, at: H256) -> Result<Hash32> {
    Ok(
        storage_decode(api, "GearEthBridge", "BridgeDomain", at, vec![])
            .await?
            .unwrap_or([0; 32]),
    )
}

// Released GearEthBridge.DestinationBinding is SCALE (H256, H256, H160).
type DestinationBinding = (Hash32, Hash32, [u8; 20]);

fn validate_destination_binding(
    genesis: Hash32,
    domain: Hash32,
    binding: Option<DestinationBinding>,
) -> Result<DestinationBinding> {
    let binding = binding.context("native source destination binding is missing; HOLD")?;
    ensure!(
        genesis != [0; 32]
            && binding.0 == genesis
            && binding.1 != [0; 32]
            && binding.2 != [0; 20]
            && domain != [0; 32],
        "native source destination binding has wrong genesis or zero identity; HOLD"
    );
    // Unlike the deployment helper, the released storage supports all 256 chain-ID bits.
    const PREFIX: &[u8] = b"vara/gear-eth-bridge-domain/v2";
    let mut preimage = [0u8; PREFIX.len() + 84];
    preimage[..PREFIX.len()].copy_from_slice(PREFIX);
    preimage[PREFIX.len()..PREFIX.len() + 32].copy_from_slice(&genesis);
    preimage[PREFIX.len() + 32..PREFIX.len() + 64].copy_from_slice(&binding.1);
    preimage[PREFIX.len() + 64..].copy_from_slice(&binding.2);
    ensure!(
        keccak256(&preimage) == domain,
        "native source destination binding differs from bridge domain; HOLD"
    );
    Ok(binding)
}

async fn discover_source_identity(api: &GearApi, at: H256) -> Result<SourceIdentity> {
    let genesis = api.block_number_to_hash(0).await?;
    let block = api.block_hash_to_number(at).await?;
    let runtime = runtime_identity_at(api, at).await?;
    validate_bridge_apis(&runtime)?;
    let bytes = api
        .api
        .legacy()
        .state_call("BeefyApi_beefy_genesis", None, Some(at))
        .await?;
    let activation = decode_exact::<Option<u32>>(&bytes)?;
    let count = mmr_leaf_count(api, at).await?;
    let (activation, start) = active_mmr_start(activation, block, count)?;
    let root = mmr_root(api, at).await?;
    ensure!(root != [0; 32], "nonempty finalized MMR has zero root");
    let (current, next) = checkpoint_at_hash(api, at, block).await?;
    let domain = bridge_domain_at(api, at).await?;
    let destination = validate_destination_binding(
        genesis.0,
        domain,
        storage_decode(api, "GearEthBridge", "DestinationBinding", at, vec![])
            .await
            .context("read native destination binding at common finalized pin; HOLD")?,
    )?;
    let mut lower = 0;
    let mut upper = block;
    while lower < upper {
        let middle = lower + (upper - lower) / 2;
        if bridge_domain_at(api, api.block_number_to_hash(middle).await?).await? == [0; 32] {
            lower = middle.checked_add(1).context("domain search overflow")?;
        } else {
            upper = middle;
        }
    }
    let binding = lower;
    let binding_at = api.block_number_to_hash(binding).await?;
    let binding_domain = bridge_domain_at(api, binding_at).await?;
    ensure!(
        binding_domain == domain,
        "source bridge domain changed since first binding; HOLD"
    );
    let historical_destination = validate_destination_binding(
        genesis.0,
        binding_domain,
        storage_decode(
            api,
            "GearEthBridge",
            "DestinationBinding",
            binding_at,
            vec![],
        )
        .await
        .context("read native destination binding at first binding pin; HOLD")?,
    )?;
    ensure!(
        historical_destination == destination,
        "native source destination binding changed since first binding; HOLD"
    );
    let first: u32 = start.try_into()?;
    let first_at = api.block_number_to_hash(first).await?;
    ensure!(
        mmr_leaf_count(api, first_at).await? == 1,
        "inferred first MMR insertion does not contain exactly one leaf"
    );
    // Read storage at S-1: that runtime may not yet expose MmrApi.
    let previous_at = api
        .block_number_to_hash(
            first
                .checked_sub(1)
                .context("first MMR insertion precedes genesis")?,
        )
        .await?;
    let previous_count: u64 = storage_decode(api, "Mmr", "NumberOfLeaves", previous_at, vec![])
        .await?
        .unwrap_or(0);
    ensure!(
        previous_count == 0,
        "MMR already contained leaves before inferred first insertion"
    );
    let response: MmrLeavesProof = api
        .api
        .rpc()
        .request(
            "mmr_generateProof",
            rpc_params![vec![first], Some(block), Some(at)],
        )
        .await?;
    ensure!(
        parse_h256(&response.block_hash)? == at.0,
        "first MMR proof anchor differs from common pin"
    );
    let raw = decode_mmr_leaves(&decode_hex_bounded(&response.leaves, MAX_MMR_LEAVES_BYTES)?)?;
    let leaf = decode_outer_leaf(&raw)?;
    let proof = decode_mmr_proof(&decode_hex_bounded(&response.proof, MAX_MMR_PROOF_BYTES)?)?;
    validate_first_leaf_geometry(first, previous_at.0, count, &leaf, &proof)?;
    let leaf_hash = outer_leaf_hash(&leaf)?;
    ensure!(
        mmr_root(api, first_at).await? == leaf_hash,
        "first insertion root differs from first leaf hash"
    );
    verify_native_mmr_proof(root, &proof, leaf_hash)
        .context("first leaf is not included in common finalized MMR root")?;
    // S can precede the session that installs real BEEFY keys. This leaf is
    // geometry evidence, not an acceptance authority set; authenticate its exact
    // historical tuple without requiring today's nonempty authority checkpoint.
    let first_next = api
        .api
        .legacy()
        .state_call("BeefyMmrApi_next_authority_set_proof", None, Some(first_at))
        .await?;
    let first_next: BeefyAuthoritySet<Hash32> = decode_exact(&first_next)?;
    ensure!(
        leaf.beefy_next_authority_set == first_next,
        "first MMR leaf authority transition differs from insertion state"
    );
    Ok(SourceIdentity {
        source_genesis: genesis.0,
        finalized_height: block,
        finalized_hash: at.0,
        runtime,
        current,
        next,
        beefy_activation_block: activation,
        mmr_start_block: start,
        bridge_domain: domain,
        domain_binding_block: binding,
        mmr_leaf_count: count,
        mmr_root: root,
        first_insertion_hash: first_at.0,
        first_leaf_hash: leaf_hash,
    })
}

fn validate_first_leaf_geometry(
    first: u32,
    previous_hash: Hash32,
    count: u64,
    leaf: &RuntimeLeaf,
    proof: &RuntimeLeafProof,
) -> Result<()> {
    ensure!(
        leaf.parent_number_and_hash
            == (
                first
                    .checked_sub(1)
                    .context("first leaf parent underflow")?,
                previous_hash
            ),
        "first MMR leaf does not commit insertion parent S-1"
    );
    ensure!(
        proof.leaf_indices == [0] && proof.leaf_count == count,
        "first MMR leaf proof count/index differs from pinned geometry"
    );
    Ok(())
}

fn validate_application_block(start: u64, binding: u32, source: u32, anchor: u32) -> Result<()> {
    ensure!(
        u64::from(source) >= start && source >= binding && source < anchor,
        "application block precedes MMR/domain admission or is not before commitment"
    );
    Ok(())
}

fn expected_leaf_count(mmr_start_block: u64, block: u32) -> Result<u64> {
    u64::from(block)
        .checked_sub(mmr_start_block)
        .and_then(|count| count.checked_add(1))
        .context("block precedes MMR start block")
}

async fn checkpoint_at(api: &GearApi, block: u32) -> Result<(AuthoritySet, AuthoritySet)> {
    let hash = api.block_number_to_hash(block).await?;
    checkpoint_at_hash(api, hash, block).await
}

async fn checkpoint_at_hash(
    api: &GearApi,
    hash: H256,
    block: u32,
) -> Result<(AuthoritySet, AuthoritySet)> {
    let validator_bytes = api
        .api
        .legacy()
        .state_call("BeefyApi_validator_set", None, Some(hash))
        .await?;
    let validator_set =
        decode_validator_set(&validator_bytes).context("decode historical BEEFY validator set")?;
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
    let next_bytes = storage_encoded(api, "Beefy", "NextAuthorities", hash, vec![])
        .await?
        .ok_or_else(|| {
            anyhow!("historical next BEEFY authority list is unavailable at block {block}")
        })?;
    let next_keys = decode_authority_list(&next_bytes)?;
    let next = AuthoritySet {
        id: next_proof.id,
        root: next_proof.keyset_commitment,
        keys: next_keys
            .iter()
            .map(|key| <AuthorityId as AsRef<[u8]>>::as_ref(key).to_vec())
            .collect(),
    };
    ensure!(
        next.id
            == current
                .id
                .checked_add(1)
                .context("BEEFY authority set id overflow")?,
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

pub async fn bridge_status(api: &GearApi, at_block: Option<u32>) -> Result<()> {
    let block = match at_block {
        Some(number) => api.block_number_to_hash(number).await?,
        None => api.latest_finalized_block().await?,
    };
    let number = api.block_hash_to_number(block).await?;
    let (queue_id, root) = api.fetch_queue_merkle_root(block).await?;
    println!(
        "block={number} hash={block:?} initialized={} paused={} queue_id={queue_id} root=0x{}",
        bridge_initialized(api, block).await?,
        bridge_paused(api, block).await?,
        hex::encode(root.0)
    );
    Ok(())
}

pub async fn runtime_identity_at(api: &GearApi, at: H256) -> Result<JsonValue> {
    let version: JsonValue = api
        .api
        .rpc()
        .request("state_getRuntimeVersion", rpc_params![at])
        .await?;
    ensure!(
        version["specName"].as_str().is_some()
            && version["specVersion"].as_u64().is_some()
            && version["apis"]
                .as_array()
                .is_some_and(|apis| !apis.is_empty()),
        "runtime/API identity is incomplete; HOLD"
    );
    let babe = api
        .api
        .legacy()
        .state_call("BabeApi_configuration", None, Some(at))
        .await?;
    ensure!(
        babe.len() >= 16,
        "BabeApi_configuration response is truncated"
    );
    let slot_duration = u64::from_le_bytes(babe[0..8].try_into()?);
    let epoch_length = u64::from_le_bytes(babe[8..16].try_into()?);
    let code: Option<String> = api
        .api
        .rpc()
        .request("state_getStorage", rpc_params!["0x3a636f6465", at])
        .await?;
    let code = decode_hex(&code.context("runtime :code is missing at requested pin")?)?;
    ensure!(!code.is_empty(), "runtime :code is empty at requested pin");
    Ok(json!({"version": version,
        "runtimeCodeSha256": format!("0x{}", hex::encode(sp_core::hashing::sha2_256(&code))),
        "runtimeCodeKeccak256": format!("0x{}", hex::encode(keccak256(&code))),
        "runtimeCodeBlake2b256": format!("0x{}", hex::encode(sp_core::blake2_256(&code))),
        "babe": {"slotDuration": slot_duration, "epochLength": epoch_length}}))
}

fn validate_bridge_apis(runtime: &JsonValue) -> Result<()> {
    let apis = runtime["version"]["apis"]
        .as_array()
        .context("runtime APIs are missing")?;
    for (name, minimum) in [("BeefyApi", 5), ("MmrApi", 2), ("BeefyMmrApi", 1)] {
        let id = format!("0x{}", hex::encode(sp_core::blake2_64(name.as_bytes())));
        ensure!(
            apis.iter()
                .any(|api| api[0] == id && api[1].as_u64().is_some_and(|v| v >= minimum)),
            "required {name} runtime API identity/version is missing; HOLD"
        );
    }
    Ok(())
}

/// The retained fast Hoodi rehearsal remains strict; normal-runtime discovery
/// does not call this genesis-only admission profile.
pub async fn validate_legacy_runtime(api: &GearApi) -> Result<JsonValue> {
    let genesis = api.block_number_to_hash(0).await?;
    let mut runtime = runtime_identity_at(api, genesis).await?;
    ensure!(
        runtime["babe"]["slotDuration"] == 3000 && runtime["babe"]["epochLength"] == 64,
        "unexpected legacy fast-runtime BABE cadence"
    );
    let activation = api
        .api
        .legacy()
        .state_call("BeefyApi_beefy_genesis", None, Some(genesis))
        .await?;
    ensure!(
        decode_exact::<Option<u32>>(&activation)? == Some(1),
        "legacy local BEEFY activation is not block one"
    );
    ensure!(
        mmr_leaf_count(api, genesis).await? == 0,
        "legacy genesis already contains MMR leaves"
    );
    let (current, next) = checkpoint_at(api, 0).await?;
    runtime["genesisHash"] = json!(format!("0x{}", hex::encode(genesis.0)));
    runtime["authorityGenesis"] = json!({"current": current, "next": next});
    Ok(runtime)
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

pub async fn bridge_initialized(api: &GearApi, at: H256) -> Result<bool> {
    Ok(
        storage_decode(api, "GearEthBridge", "Initialized", at, vec![])
            .await?
            .unwrap_or(false),
    )
}

/// Match the native pallet's bridge_snapshot: an uninitialized bridge has no
/// queue snapshot, not an initialized zero root. Once initialized, the root key
/// must exist; RPC absence or malformed storage remains an error.
pub async fn queue_snapshot_at(api: &GearApi, at: H256) -> Result<Option<(u64, H256)>> {
    if !bridge_initialized(api, at)
        .await
        .with_context(|| format!("read bridge initialization at {at:?}; HOLD"))?
    {
        return Ok(None);
    }
    Ok(Some(api.fetch_queue_merkle_root(at).await.with_context(
        || format!("read initialized bridge queue at {at:?}; HOLD"),
    )?))
}

pub async fn bridge_paused(api: &GearApi, at: H256) -> Result<bool> {
    Ok(storage_decode(api, "GearEthBridge", "Paused", at, vec![])
        .await?
        .unwrap_or(true))
}

pub async fn bridge_events(api: &GearApi, at_block: u32, full: bool) -> Result<()> {
    let block = api.block_number_to_hash(at_block).await?;
    let events = api.get_block_at(block).await?.events().await?;
    for event in events.iter() {
        let event = event?;
        if event.pallet_name() == "Gear"
            && matches!(event.variant_name(), "MessageQueued" | "UserMessageSent")
        {
            let bytes = event.field_bytes();
            println!(
                "Gear::{} field_bytes={} prefix=0x{}",
                event.variant_name(),
                bytes.len(),
                hex::encode(&bytes[..bytes.len().min(if full { bytes.len() } else { 128 })])
            );
        }
    }
    Ok(())
}
pub async fn has_event(api: &GearApi, at: H256, pallet: &str, variant: &str) -> Result<bool> {
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

async fn storage_decode<T: Decode + Encode>(
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
    Ok(Some(decode_exact(value.encoded()).with_context(|| {
        format!("decode {pallet}.{entry} storage")
    })?))
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

fn authority_set_root(keys: &[Vec<u8>]) -> Result<Hash32> {
    let keys = key_arrays(keys)?;
    authority_root(&authority_addresses(&keys)?)
}

fn key_arrays(keys: &[Vec<u8>]) -> Result<Vec<[u8; 33]>> {
    ensure!(
        (1..=MAX_VALIDATORS).contains(&keys.len()),
        "invalid authority count"
    );
    keys.iter()
        .map(|key| {
            key.as_slice()
                .try_into()
                .map_err(|_| anyhow!("authority key is not 33 bytes"))
        })
        .collect()
}

fn decode_exact<T: Decode + Encode>(bytes: &[u8]) -> Result<T> {
    ensure!(
        bytes.len() <= MAX_VALIDATORS * 33 + 32,
        "SCALE value exceeds body limit"
    );
    let mut input = bytes;
    let value = T::decode(&mut input)?;
    ensure!(input.is_empty(), "SCALE value has trailing bytes");
    ensure!(value.encode() == bytes, "SCALE value is not canonical");
    Ok(value)
}

#[cfg(any(test, feature = "deployment"))]
fn decode_session_validators(bytes: &[u8]) -> Result<Vec<[u8; 32]>> {
    let mut input = bytes;
    let len = Compact::<u32>::decode(&mut input)?.0 as usize;
    ensure!(
        len <= MAX_VALIDATORS && input.len() == len * 32,
        "invalid session validator vector"
    );
    decode_exact(bytes)
}

fn decode_hex_bounded(value: &str, max: usize) -> Result<Vec<u8>> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    ensure!(
        value.len().is_multiple_of(2) && value.len() / 2 <= max,
        "RPC hex exceeds body limit or has odd length"
    );
    hex::decode(value).context("decode RPC hex")
}

fn decode_hex(value: &str) -> Result<Vec<u8>> {
    // gsdk 1.10/jsonrpsee bounds each response frame to 10 MiB before JSON decoding.
    // Keep runtime :code retrieval within the same bound; proofs use tighter limits.
    decode_hex_bounded(value, 10 * 1024 * 1024)
}

fn parse_h256(value: &str) -> Result<Hash32> {
    let value = value.strip_prefix("0x").unwrap_or(value);
    ensure!(value.len() == 64, "RPC hash is not 32 bytes");
    let mut bytes = [0; 32];
    hex::decode_to_slice(value, &mut bytes).context("decode RPC hash")?;
    Ok(bytes)
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
        let encoded_engine = pair[0].as_array().context("invalid consensus engine id")?;
        ensure!(
            encoded_engine.len() == 4,
            "invalid consensus engine id width"
        );
        let mut engine = [0; 4];
        for (target, byte) in engine.iter_mut().zip(encoded_engine) {
            *target = byte
                .as_u64()
                .and_then(|byte| u8::try_from(byte).ok())
                .context("invalid consensus engine id byte")?;
        }
        if engine == BEEFY_ENGINE {
            ensure!(found.is_none(), "duplicate BEEFY engine justification");
            let bytes = pair[1]
                .as_array()
                .context("invalid BEEFY justification byte array")?;
            ensure!(
                bytes.len() <= MAX_BEEFY_PROOF_BYTES,
                "BEEFY justification exceeds body limit"
            );
            found = Some(
                bytes
                    .iter()
                    .map(|byte| {
                        byte.as_u64()
                            .and_then(|byte| u8::try_from(byte).ok())
                            .context("invalid BEEFY justification byte")
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_rewinds_proofless_scan_without_losing_pending_or_delivered_originals() -> Result<()>
    {
        let pending = (101..100 + MAX_PENDING_JUSTIFICATIONS as u32)
            .map(|block| (block, block.to_le_bytes().to_vec()))
            .collect();
        let saved = ScanJournal {
            genesis: [1; 32],
            bridge_domain: [2; 32],
            scanned_through: 2_000,
            scanned_hash: [3; 32],
            pending,
            delivered: Some((100, vec![4; 32])),
        };
        let mut restored: ScanJournal = serde_json::from_slice(&serde_json::to_vec(&saved)?)?;
        restored.rewind_to(50, [5; 32]);
        assert_eq!(restored.scanned_through, 50);
        assert_eq!(restored.scanned_hash, [5; 32]);
        assert_eq!(restored.pending, saved.pending);
        assert_eq!(restored.delivered, saved.delivered);
        // Both transitions appended after the old scan must be visited before
        // the later retained originals can be delivered.
        let replay = restored.scanned_through + 1..=saved.scanned_through;
        assert!(replay.contains(&60) && replay.contains(&80));
        Ok(())
    }

    #[test]
    fn native_destination_binding_authenticates_genesis_chain_id_queue_and_domain() -> Result<()> {
        let genesis = [0x22; 32];
        let queue = [0x33; 20];
        let mut chain_id = [0; 32];
        chain_id[24..].copy_from_slice(&31_337u64.to_be_bytes());
        let binding = (genesis, chain_id, queue);
        // Existing runtime/protocol vector: chain ID is 32-byte big endian, not SCALE u64.
        let domain =
            parse_h256("9aac6d72e183672d20696112082accb15719870152120212f541e3e233837944")?;
        assert_eq!(
            validate_destination_binding(genesis, domain, Some(binding))?,
            binding
        );
        for invalid in [
            None,
            Some(([0; 32], chain_id, queue)),
            Some(([0x23; 32], chain_id, queue)),
            Some((genesis, [0; 32], queue)),
            Some((genesis, chain_id, [0; 20])),
            Some((genesis, chain_id, [0x34; 20])),
        ] {
            assert!(validate_destination_binding(genesis, domain, invalid).is_err());
        }
        assert!(validate_destination_binding([0; 32], domain, Some(binding)).is_err());
        assert!(validate_destination_binding([0x23; 32], domain, Some(binding)).is_err());
        assert!(validate_destination_binding(genesis, [0; 32], Some(binding)).is_err());
        assert!(validate_destination_binding(genesis, [1; 32], Some(binding)).is_err());
        chain_id[24..].copy_from_slice(&31_337u64.to_le_bytes());
        assert!(
            validate_destination_binding(genesis, domain, Some((genesis, chain_id, queue)))
                .is_err()
        );
        Ok(())
    }

    #[test]
    fn native_destination_binding_decodes_exact_released_tuple_without_chain_id_truncation(
    ) -> Result<()> {
        let genesis = [0x22; 32];
        let mut chain_id = [0; 32];
        chain_id[0] = 0x80; // A valid nonzero chain ID whose low 64 bits are zero.
        let queue = [0x33; 20];
        let bytes = (
            sp_core::H256::from(genesis),
            sp_core::H256::from(chain_id),
            sp_core::H160::from(queue),
        )
            .encode();
        assert_eq!(bytes.len(), 84);
        let binding = decode_exact::<DestinationBinding>(&bytes)?;
        assert_eq!(binding, (genesis, chain_id, queue));
        let preimage = [
            b"vara/gear-eth-bridge-domain/v2".as_slice(),
            bytes.as_slice(),
        ]
        .concat();
        let domain = sp_core::keccak_256(&preimage);
        assert_eq!(
            validate_destination_binding(genesis, domain, Some(binding))?,
            binding
        );
        let mut changed = binding;
        changed.1[0] ^= 1;
        assert!(validate_destination_binding(genesis, domain, Some(changed)).is_err());
        assert!(decode_exact::<DestinationBinding>(&bytes[..83]).is_err());
        let mut trailing = bytes;
        trailing.push(0);
        assert!(decode_exact::<DestinationBinding>(&trailing).is_err());
        Ok(())
    }

    #[test]
    fn attachment_preserves_labeled_runtime_hashes_not_artifact_or_genesis_guesses() {
        let runtime = json!({"runtimeCodeSha256": format!("0x{}", "11".repeat(32)),
            "runtimeCodeKeccak256": format!("0x{}", "22".repeat(32)),
            "runtimeCodeBlake2b256": format!("0x{}", "33".repeat(32))});
        assert!(ensure_runtime_binding(&runtime, &runtime).is_ok());
        let mut upgraded = runtime.clone();
        upgraded["runtimeCodeSha256"] = json!(format!("0x{}", "44".repeat(32)));
        assert!(ensure_runtime_binding(&runtime, &upgraded).is_err());
        let mut mislabeled = runtime.clone();
        mislabeled["runtimeCodeSha256"] = runtime["runtimeCodeKeccak256"].clone();
        assert!(ensure_runtime_binding(&mislabeled, &runtime).is_err());
        assert!(ensure_runtime_binding(
            &json!({"runtimeCodeHash": runtime["runtimeCodeSha256"]}),
            &runtime
        )
        .is_err());
    }

    #[test]
    fn common_pin_requires_the_production_bridge_runtime_api_versions() {
        let apis = [("BeefyApi", 5), ("MmrApi", 2), ("BeefyMmrApi", 1)].map(|(name, version)| {
            json!([
                format!("0x{}", hex::encode(sp_core::blake2_64(name.as_bytes()))),
                version
            ])
        });
        let runtime = json!({"version": {"apis": apis}});
        assert!(validate_bridge_apis(&runtime).is_ok());
        let mut old = runtime.clone();
        old["version"]["apis"][0][1] = json!(4);
        assert!(validate_bridge_apis(&old).is_err());
        let mut missing = runtime.clone();
        missing["version"]["apis"].as_array_mut().unwrap().pop();
        assert!(validate_bridge_apis(&missing).is_err());
    }

    #[test]
    fn delayed_activation_is_pending_not_genesis_or_first_leaf_default() {
        for (activation, count) in [
            (None, 0),
            (None, 10),
            (Some(0), 10),
            (Some(101), 10),
            (Some(50), 0),
        ] {
            assert!(active_mmr_start(activation, 100, count)
                .unwrap_err()
                .is::<SourceActivationPending>());
        }
        assert_eq!(active_mmr_start(Some(90), 100, 21).unwrap(), (90, 80));
        assert!(active_mmr_start(Some(90), 100, 101).is_err());
    }

    #[test]
    fn upgraded_common_pin_compares_every_authenticated_identity() -> Result<()> {
        let authority = AuthoritySet {
            id: 1,
            keys: vec![vec![2; 33]],
            root: [3; 32],
        };
        let identity = SourceIdentity {
            source_genesis: [1; 32],
            finalized_height: 100,
            finalized_hash: [2; 32],
            runtime: json!({"runtimeCodeSha256": "qualified upgraded code", "babe": {"epochLength": 2400}}),
            current: authority.clone(),
            next: AuthoritySet { id: 2, ..authority },
            beefy_activation_block: 90,
            mmr_start_block: 80,
            bridge_domain: [4; 32],
            domain_binding_block: 85,
            mmr_leaf_count: 21,
            mmr_root: [5; 32],
            first_insertion_hash: [6; 32],
            first_leaf_hash: [7; 32],
        };
        ensure_paired_identity(&identity, &identity)?;
        let mut changed = identity.clone();
        changed.runtime["babe"]["epochLength"] = json!(64);
        assert!(ensure_paired_identity(&identity, &changed).is_err());
        let mut changed = identity.clone();
        changed.finalized_hash[0] ^= 1;
        assert!(ensure_paired_identity(&identity, &changed).is_err());
        let mut changed = identity.clone();
        changed.next.root[0] ^= 1;
        assert!(ensure_paired_identity(&identity, &changed).is_err());
        let mut changed = identity.clone();
        changed.beefy_activation_block = 1;
        assert!(ensure_paired_identity(&identity, &changed).is_err());
        let mut changed = identity.clone();
        changed.mmr_start_block = 1;
        assert!(ensure_paired_identity(&identity, &changed).is_err());
        let mut changed = identity.clone();
        changed.domain_binding_block = 0;
        assert!(ensure_paired_identity(&identity, &changed).is_err());
        let mut changed = identity.clone();
        changed.mmr_root[0] ^= 1;
        assert!(ensure_paired_identity(&identity, &changed).is_err());
        Ok(())
    }

    #[test]
    fn first_leaf_commits_s_minus_one_without_admitting_its_application_history() -> Result<()> {
        let leaf = RuntimeLeaf {
            version: sp_consensus_beefy::mmr::MmrLeafVersion::new(0, 0),
            parent_number_and_hash: (79, [2; 32]),
            beefy_next_authority_set: BeefyAuthoritySet {
                id: 2,
                len: 2,
                keyset_commitment: [3; 32],
            },
            leaf_extra: [4; 32],
        };
        let proof = RuntimeLeafProof {
            leaf_indices: vec![0],
            leaf_count: 21,
            items: vec![],
        };
        validate_first_leaf_geometry(80, [2; 32], 21, &leaf, &proof)?;
        assert!(validate_first_leaf_geometry(81, [2; 32], 21, &leaf, &proof).is_err());
        assert!(validate_first_leaf_geometry(80, [9; 32], 21, &leaf, &proof).is_err());
        let mut bad = proof.clone();
        bad.leaf_count = 20;
        assert!(validate_first_leaf_geometry(80, [2; 32], 21, &leaf, &bad).is_err());
        let mut bad = proof.clone();
        bad.leaf_indices[0] = 1;
        assert!(validate_first_leaf_geometry(80, [2; 32], 21, &leaf, &bad).is_err());
        assert!(validate_application_block(80, 0, 79, 100).is_err());
        assert!(validate_application_block(80, 85, 80, 100).is_err());
        assert!(validate_application_block(80, 85, 85, 100).is_ok());
        assert!(validate_application_block(80, 85, 100, 100).is_err());
        assert_eq!(expected_leaf_count(80, 100)?, 21);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires independent archived source/witness RPCs; read-only, no activation"]
    async fn live_common_finalized_identity_and_first_insertion() -> Result<()> {
        let rpc = std::env::var("SOURCE_IDENTITY_RPC")?;
        let witness = std::env::var("SOURCE_IDENTITY_WITNESS_RPC")?;
        ensure!(
            rpc != witness && crate::local_source_rpc(&rpc) && crate::local_source_rpc(&witness),
            "independent local read-only endpoints required"
        );
        let (source, witness) = Source::connect_pair(
            GearApi::new(rpc.as_str(), 3).await?,
            GearApi::new(witness.as_str(), 3).await?,
        )
        .await?;
        ensure_paired_identity(&source.identity, &witness.identity)?;
        println!("{}", serde_json::to_string(&source.identity)?);
        Ok(())
    }

    #[test]
    fn rpc_hex_and_justifications_are_bounded_before_byte_allocation() {
        assert!(decode_hex_bounded("0x1234", 1).is_err());
        assert!(decode_hex_bounded("0x1", 1).is_err());
        assert_eq!(decode_hex_bounded("0x12", 1).unwrap(), vec![0x12]);
        assert!(parse_h256(&"00".repeat(33)).is_err());
        assert_eq!(parse_h256(&"00".repeat(32)).unwrap(), [0; 32]);
        let oversized =
            json!({"justifications": [[BEEFY_ENGINE, vec![0; MAX_BEEFY_PROOF_BYTES + 1]]]});
        assert!(justification_bytes(&oversized).is_err());
        assert!(justification_bytes(&json!({"justifications": [[BEEFY_ENGINE, [256]]]})).is_err());
        assert_eq!(
            justification_bytes(&json!({"justifications": [[BEEFY_ENGINE, [1, 2]]]})).unwrap(),
            Some(vec![1, 2])
        );
        assert!(decode_session_validators(&Compact(u32::MAX).encode()).is_err());
        assert!(decode_session_validators(&vec![[0u8; 32]; 257].encode()).is_err());
        assert_eq!(
            decode_session_validators(&vec![[0u8; 32]; 256].encode())
                .unwrap()
                .len(),
            256
        );
    }

    #[test]
    fn alternate_quorate_subsets_share_semantics_but_still_require_valid_signatures() -> Result<()>
    {
        use sp_consensus_beefy::VersionedFinalityProof;
        let fixture = crate::fixtures::generate_fixtures()?;
        let case = fixture
            .cases
            .iter()
            .find(|case| case.validator_count == 59)
            .context("59-validator fixture")?;
        let raw = hex::decode(case.signed_commitment.trim_start_matches("0x"))?;
        let keys: Vec<[u8; 33]> = case
            .authority_keys
            .iter()
            .map(|key| {
                hex::decode(key.trim_start_matches("0x"))?
                    .try_into()
                    .map_err(|_| anyhow!("invalid fixture key"))
            })
            .collect::<Result<_>>()?;
        let original =
            validate_signed_commitment(&raw, &keys, Some(0), Some(case.anchor_block as u32))?;
        let mut subset = decode_versioned_finality_proof(&raw)?;
        subset.signatures[0] = None;
        let subset = VersionedFinalityProof::V1(subset).encode();
        let alternate =
            validate_signed_commitment(&subset, &keys, Some(0), Some(case.anchor_block as u32))?;
        assert_ne!(raw, subset);
        assert_eq!(commitment_identity(&raw)?, commitment_identity(&subset)?);
        assert_eq!(original.commitment_bytes, alternate.commitment_bytes);
        let mut invalid = decode_versioned_finality_proof(&subset)?;
        invalid.commitment.block_number += 1;
        let invalid = VersionedFinalityProof::V1(invalid).encode();
        assert_ne!(commitment_identity(&raw)?, commitment_identity(&invalid)?);
        assert!(validate_signed_commitment(
            &invalid,
            &keys,
            Some(0),
            Some(case.anchor_block as u32 + 1)
        )
        .is_err());
        let mut undersigned = decode_versioned_finality_proof(&raw)?;
        for signature in undersigned.signatures.iter_mut().skip(1) {
            *signature = None;
        }
        let undersigned = VersionedFinalityProof::V1(undersigned).encode();
        assert_eq!(
            commitment_identity(&raw)?,
            commitment_identity(&undersigned)?
        );
        assert!(validate_signed_commitment(
            &undersigned,
            &keys,
            Some(0),
            Some(case.anchor_block as u32)
        )
        .is_err());
        Ok(())
    }

    #[test]
    fn source_range_uses_first_mmr_block() {
        assert_eq!(expected_leaf_count(1, 1).unwrap(), 1);
        assert_eq!(expected_leaf_count(1_000, 1_002).unwrap(), 3);
        assert!(expected_leaf_count(1_000, 999).is_err());
    }

    #[tokio::test]
    #[ignore = "requires an independently witnessed archived source chain and its original resume/first heights"]
    async fn live_catchup_replays_original_handoff_and_returns_consecutive_handovers() -> Result<()>
    {
        let rpc = std::env::var("SOURCE_CATCHUP_RPC")?;
        let witness_rpc = std::env::var("SOURCE_CATCHUP_WITNESS_RPC")?;
        ensure!(
            crate::local_source_rpc(&rpc) && crate::local_source_rpc(&witness_rpc),
            "live source recovery only reads loopback endpoints"
        );
        let genesis: Hash32 = decode_hex(&std::env::var("SOURCE_CATCHUP_GENESIS")?)?
            .try_into()
            .map_err(|_| anyhow!("source genesis must contain 32 bytes"))?;
        let resume: u32 = std::env::var("SOURCE_CATCHUP_RESUME_FROM")?.parse()?;
        let expected_first: u32 = std::env::var("SOURCE_CATCHUP_EXPECT_FIRST")?.parse()?;
        ensure!(
            expected_first > resume,
            "first expected commitment must follow the saved checkpoint"
        );
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("source-scan.json");
        let original_cursor =
            if let Some(original) = std::env::var_os("SOURCE_CATCHUP_SCAN_JOURNAL") {
                let bytes = fs::read(original)?;
                let saved: ScanJournal = serde_json::from_slice(&bytes)?;
                ensure!(
                    saved.pending.len() == MAX_PENDING_JUSTIFICATIONS,
                    "fixture must reproduce a saturated original backlog"
                );
                fs::write(&path, bytes)?;
                Some(saved.scanned_through)
            } else {
                None
            };
        let started = std::time::Instant::now();
        timeout(Duration::from_secs(45), async {
            let (mut source, witness) = Source::connect_pair(
                GearApi::new(rpc.as_str(), 3).await?,
                GearApi::new(witness_rpc.as_str(), 3).await?,
            )
            .await?;
            assert_eq!(source.source_genesis, genesis);
            assert_eq!(witness.source_genesis, genesis);
            assert_eq!(source.bridge_domain, witness.bridge_domain);
            if original_cursor.is_none() {
                // Reproduce a cursor persisted while GRANDPA-finalized history
                // still had no BEEFY proofs. The archive now contains them.
                let saved = ScanJournal {
                    genesis: source.source_genesis,
                    bridge_domain: source.bridge_domain,
                    scanned_through: source.identity.finalized_height,
                    scanned_hash: source.identity.finalized_hash,
                    pending: Default::default(),
                    delivered: None,
                };
                fs::write(&path, serde_json::to_vec(&saved)?)?;
            }
            source.seek_from(resume.checked_add(1).context("resume height overflow")?)?;
            source.enable_scan_journal(&path, resume, &witness).await?;
            assert_eq!(source.scanned_through, resume);
            let first = source.next_commitment().await?;
            assert_eq!(
                first.block, expected_first,
                "catch-up must return the original earliest commitment"
            );
            assert_eq!(
                source.scanned_through, expected_first,
                "restart must rescan earlier history before draining later originals"
            );
            let saved: ScanJournal = serde_json::from_slice(&fs::read(&path)?)?;
            assert_eq!(saved.scanned_through, expected_first);
            assert_eq!(
                saved.delivered.as_ref().map(|(block, _)| *block),
                Some(expected_first)
            );
            assert!(
                first.block < first.finalized_height,
                "fixture must retain historical backlog"
            );
            let witnessed = witness.recapture(first.block, first.raw.clone()).await?;
            assert_eq!(witnessed.block_hash, first.block_hash);
            assert_eq!(
                witnessed.validated.commitment_bytes,
                first.validated.commitment_bytes
            );
            println!(
                "source-catchup first={} finalized={} elapsed_ms={}",
                first.block,
                first.finalized_height,
                started.elapsed().as_millis()
            );
            println!(
                "source-catchup original_scan={}",
                fs::read_to_string(&path)?
            );

            // Lose the caller before it journals acceptance; only the scan file survives.
            drop(source);
            let mut source = Source::connect(GearApi::new(rpc.as_str(), 3).await?).await?;
            source.seek_from(resume + 1)?;
            source.enable_scan_journal(&path, resume, &witness).await?;
            let replayed = source.next_commitment().await?;
            assert_eq!(replayed.block, first.block);
            assert_eq!(replayed.block_hash, first.block_hash);
            assert_eq!(
                replayed.raw, first.raw,
                "restart must retain the original signed proof bytes"
            );
            drop(source);

            // Once the original is accepted, reconnect must not emit it again or skip a set.
            let mut source = Source::connect(GearApi::new(rpc.as_str(), 3).await?).await?;
            source.seek_from(
                first
                    .block
                    .checked_add(1)
                    .context("accepted height overflow")?,
            )?;
            source
                .enable_scan_journal(&path, first.block, &witness)
                .await?;
            let mut previous_block = first.block;
            let mut previous_set = first.validated.signed.commitment.validator_set_id;
            let mut handovers = 0;
            while handovers < 2 {
                let next = source.next_commitment().await?;
                assert!(
                    next.block > previous_block,
                    "accepted source history must advance strictly"
                );
                let set = next.validated.signed.commitment.validator_set_id;
                if set != previous_set {
                    assert_eq!(
                        set,
                        previous_set
                            .checked_add(1)
                            .context("authority set overflow")?
                    );
                    handovers += 1;
                }
                let witnessed = witness.recapture(next.block, next.raw.clone()).await?;
                assert_eq!(witnessed.block_hash, next.block_hash);
                assert_eq!(
                    witnessed.validated.commitment_bytes,
                    next.validated.commitment_bytes
                );
                println!(
                    "source-catchup block={} set={} handovers={} elapsed_ms={}",
                    next.block,
                    set,
                    handovers,
                    started.elapsed().as_millis()
                );
                previous_block = next.block;
                previous_set = set;
            }
            println!("source-catchup final_scan={}", fs::read_to_string(&path)?);
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("real source catch-up/recovery exceeded the original 45-second bound")??;
        Ok(())
    }
}

#[cfg(any(test, feature = "deployment"))]
pub mod local {
    include!("local_source.rs");
}
