#[cfg(test)]
use crate::encode_queue_proof;
use crate::{
    outer_leaf_hash,
    source::{CapturedCommitment, SourceProof},
};
#[cfg(test)]
use anyhow::ensure;
use anyhow::Result;
use serde_json::{json, Value};
fn bytes(value: impl AsRef<[u8]>) -> String {
    format!("0x{}", hex::encode(value))
}
pub(crate) fn proof_evidence(
    proof: &SourceProof,
    anchor: &CapturedCommitment,
    encoded: &[u8],
) -> Value {
    json!({
        "sourceBlock": proof.source,
        "sourceHash": bytes(proof.source_hash),
        "insertionBlock": u64::from(proof.source) + 1,
        "anchorBlock": anchor.block,
        "anchorHash": bytes(anchor.block_hash),
        "anchorRoot": bytes(anchor.validated.mmr_root),
    "bridgeDomain": bytes(proof.snapshot.bridge_domain),
        "sourceTimestampMs": proof.snapshot.source_timestamp_ms,
        "initialized": proof.snapshot.initialized,
        "queueId": proof.snapshot.queue_id,
        "queueRoot": bytes(proof.snapshot.queue_root),
        "snapshotPreimage": bytes(proof.snapshot.encode()),
        "outerLeaf": bytes(&proof.raw_leaf),
        "outerLeafHash": bytes(outer_leaf_hash(&proof.leaf).expect("source leaf was validated")),
        "leafIndex": proof.proof.leaf_indices[0],
        "leafCount": proof.proof.leaf_count,
        "mmrItems": proof.proof.items.iter().map(bytes).collect::<Vec<_>>(),
        "simplifiedItems": proof.simplified.items.iter().map(bytes).collect::<Vec<_>>(),
        "proofOrder": bytes(proof.simplified.proof_order),
        "queueProof": bytes(encoded),
    })
}

pub(crate) fn freshness_evidence(proof: &SourceProof) -> Result<Value> {
    Ok(json!({
    "bridgeDomain": bytes(proof.snapshot.bridge_domain),
        "sourceTimestampMs": proof.snapshot.source_timestamp_ms,
        "initialized": proof.snapshot.initialized,
        "queueId": proof.snapshot.queue_id,
        "queueRoot": bytes(proof.snapshot.queue_root),
        "snapshotPreimage": bytes(proof.snapshot.encode()),
        "bridgeCommitment": bytes(proof.snapshot.hash()),
        "outerLeaf": bytes(&proof.raw_leaf),
        "outerLeafHash": bytes(outer_leaf_hash(&proof.leaf)?),
        "leafIndex": proof.proof.leaf_indices[0],
        "leafCount": proof.proof.leaf_count,
        "mmrItems": proof.proof.items.iter().map(bytes).collect::<Vec<_>>(),
        "simplifiedItems": proof.simplified.items.iter().map(bytes).collect::<Vec<_>>(),
        "proofOrder": bytes(proof.simplified.proof_order),
    }))
}

#[cfg(test)]
pub(crate) fn queue_envelope(
    proof: &SourceProof,
    anchor: &CapturedCommitment,
    message: &crate::source::local::ObservedMessage,
) -> Result<Vec<u8>> {
    ensure!(
        proof.source == message.block && proof.source_hash == message.block_hash,
        "message source identity changed"
    );
    ensure!(
        proof.snapshot == message.snapshot && proof.leaf.leaf_extra == message.snapshot.hash(),
        "MMR leaf does not authenticate the historical message queue snapshot"
    );
    ensure!(
        proof.snapshot.initialized && proof.snapshot.queue_root != [0; 32],
        "queue is not initialized and nonempty"
    );
    encode_queue_proof(
        &proof.snapshot,
        u64::from(anchor.block),
        anchor.validated.mmr_root,
        &proof.leaf,
        &proof.simplified.items,
        proof.simplified.proof_order,
    )
}
