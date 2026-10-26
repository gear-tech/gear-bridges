//! Public mainnet captures qualify proof composition, not a trusted sync-committee bootstrap.
//! The immutable response hashes and acquisition provenance live beside the shared JS fixtures.
use alloy::{
    consensus::{proofs::calculate_receipt_root, Header, TxEnvelope},
    eips::{Decodable2718, Encodable2718},
    primitives::{keccak256, B256},
    rpc::types::TransactionReceipt,
};
use anyhow::{ensure, Context, Result};
use ethereum_common::{beacon, tree_hash::TreeHash, utils};
use serde_json::Value;
use std::{fs, path::PathBuf};

use crate::inclusion::{transaction_proof, verify_indexed_value, RecordedReceiptProof};

fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/src/checkpoint_light_client/chain-data")
}

fn read_capture(fixture: &Value, name: &str) -> Result<Value> {
    let file = &fixture["files"][name];
    let bytes = fs::read(directory().join(file["path"].as_str().context("fixture path")?))?;
    ensure!(
        alloy::hex::encode(sp_core::hashing::sha2_256(&bytes)) == file["sha256"],
        "capture digest changed"
    );
    Ok(serde_json::from_slice(&bytes)?)
}

// Fulu retains Electra's BeaconBlock schema, including the 17-field Deneb execution payload.
fn validate_composition(block: &Value, root: B256, proof: &RecordedReceiptProof) -> Result<()> {
    let full: beacon::electra::Block = serde::Deserialize::deserialize(&block["data"]["message"])?;
    ensure!(
        full.tree_hash_root().as_bytes() == root.as_slice(),
        "Beacon root changed"
    );
    let light: beacon::light::electra::Block = full.into();
    ensure!(
        light.tree_hash_root().as_bytes() == root.as_slice(),
        "light Beacon root changed"
    );
    let payload = &light.body.execution_payload;
    ensure!(
        payload.block_hash.as_ref() == proof.header.hash_slow().as_slice()
            && payload.block_number == proof.header.number
            && payload.receipts_root.as_ref() == proof.header.receipts_root.as_slice(),
        "execution RLP/payload binding changed"
    );
    verify_indexed_value(
        proof.header.transactions_root,
        proof.index,
        &proof.raw_transaction,
        &proof.transaction_proof,
    )?;
    verify_indexed_value(
        proof.header.receipts_root,
        proof.index,
        &proof.receipt.encoded_2718(),
        &proof.receipt_proof,
    )
}

#[test]
fn genuine_electra_fulu_nonempty_requests_composition_and_archive_reload() -> Result<()> {
    // Full blocks contain large bounded SSZ lists; match the existing large-stack fixture convention.
    std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024)
        .spawn(run_fixtures)?
        .join()
        .unwrap()
}

fn run_fixtures() -> Result<()> {
    let provenance: Value = serde_json::from_slice(&fs::read(
        directory().join("execution-requests-provenance.json"),
    )?)?;
    let genesis: B256 = provenance["genesis_hash"]
        .as_str()
        .context("genesis")?
        .parse()?;
    for fixture in provenance["fixtures"].as_array().context("fixtures")? {
        let block = read_capture(fixture, "beacon_block")?;
        let beacon_header = read_capture(fixture, "beacon_header")?;
        let execution = read_capture(fixture, "execution_block")?;
        let receipts = read_capture(fixture, "receipts")?;
        ensure!(matches!(fixture["fork"].as_str(), Some("electra" | "fulu")));
        assert_eq!(block["version"], fixture["fork"]);
        assert_eq!(block["finalized"], true);
        assert_eq!(block["execution_optimistic"], false);
        assert_eq!(beacon_header["finalized"], true);
        assert_eq!(beacon_header["data"]["canonical"], true);
        assert_eq!(beacon_header["data"]["root"], fixture["beacon_root"]);
        let root: B256 = fixture["beacon_root"].as_str().context("root")?.parse()?;
        let header: beacon::BlockHeader =
            serde::Deserialize::deserialize(&beacon_header["data"]["header"]["message"])?;
        assert_eq!(header.slot, fixture["slot"].as_u64().unwrap());
        assert_eq!(header.tree_hash_root().as_bytes(), root.as_slice());
        let body = &block["data"]["message"]["body"];
        let payload = &body["execution_payload"];
        assert_eq!(payload.as_object().unwrap().len(), 17);
        assert!(payload.get("requests_hash").is_none());
        let requests = &body["execution_requests"];
        for kind in ["deposits", "withdrawals", "consolidations"] {
            assert_eq!(
                requests[kind].as_array().unwrap().len() as u64,
                fixture["request_counts"][kind]
            );
        }
        // Both captures contain one real consolidation. EIP-7685 hashes the request type
        // followed by the fixed-width SSZ bytes; empty request types are omitted.
        assert!(requests["deposits"].as_array().unwrap().is_empty());
        assert!(requests["withdrawals"].as_array().unwrap().is_empty());
        assert_eq!(requests["consolidations"].as_array().unwrap().len(), 1);
        let request = &requests["consolidations"][0];
        let mut encoded_request = vec![2u8];
        for (field, length) in [
            ("source_address", 20),
            ("source_pubkey", 48),
            ("target_pubkey", 48),
        ] {
            let bytes =
                alloy::hex::decode(request[field].as_str().context("consolidation field")?)?;
            assert_eq!(bytes.len(), length);
            encoded_request.extend(bytes);
        }
        let requests_hash = B256::from(sp_core::hashing::sha2_256(&sp_core::hashing::sha2_256(
            &encoded_request,
        )));
        let el_header: Header = serde::Deserialize::deserialize(&execution["result"])?;
        assert_eq!(el_header.requests_hash, Some(requests_hash));
        let execution_hash: B256 = fixture["execution_hash"].as_str().unwrap().parse()?;
        assert_eq!(el_header.hash_slow(), execution_hash);
        assert_eq!(
            el_header.number,
            fixture["execution_number"].as_u64().unwrap()
        );
        assert_eq!(execution["result"]["hash"], fixture["execution_hash"]);
        assert_eq!(payload["block_hash"], fixture["execution_hash"]);
        let transactions = payload["transactions"]
            .as_array()
            .context("transactions")?
            .iter()
            .map(|raw| {
                let bytes = alloy::hex::decode(raw.as_str().context("raw transaction")?)?;
                let mut input = bytes.as_slice();
                let transaction = TxEnvelope::decode_2718(&mut input)?;
                ensure!(
                    input.is_empty() && transaction.encoded_2718() == bytes,
                    "noncanonical transaction"
                );
                Ok(transaction)
            })
            .collect::<Result<Vec<_>>>()?;
        let receipts: Vec<TransactionReceipt> =
            serde::Deserialize::deserialize(&receipts["result"])?;
        assert_eq!(transactions.len(), receipts.len());
        assert_eq!(
            transactions.len(),
            execution["result"]["transactions"]
                .as_array()
                .unwrap()
                .len()
        );
        let mut envelopes = Vec::with_capacity(receipts.len());
        for (index, receipt) in receipts.iter().enumerate() {
            let hash = keccak256(transactions[index].encoded_2718());
            assert_eq!(
                execution["result"]["transactions"][index],
                format!("{hash:#x}")
            );
            assert_eq!(receipt.transaction_hash, hash);
            assert_eq!(receipt.transaction_index, Some(index as u64));
            assert_eq!(receipt.block_number, Some(el_header.number));
            assert_eq!(receipt.block_hash, Some(execution_hash));
            envelopes.push((index as u64, utils::map_receipt_envelope(receipt.as_ref())));
        }
        assert_eq!(
            calculate_receipt_root(&envelopes.iter().map(|(_, r)| r.clone()).collect::<Vec<_>>()),
            el_header.receipts_root
        );
        let index = fixture["selected_transaction_index"].as_u64().unwrap();
        let receipt = utils::generate_merkle_proof(index, &envelopes)?;
        let proof = RecordedReceiptProof {
            genesis,
            transaction_proof: transaction_proof(
                &transactions,
                index,
                el_header.transactions_root,
            )?,
            header: el_header,
            index,
            raw_transaction: transactions[index as usize].encoded_2718(),
            receipt: receipt.receipt,
            receipt_proof: receipt.proof,
        };
        validate_composition(&block, root, &proof)?;
        let captured_proof = read_capture(fixture, "inclusion_proof")?;
        assert_eq!(
            captured_proof["transaction_root"],
            format!("{:#x}", proof.header.transactions_root)
        );
        assert_eq!(
            captured_proof["receipt_root"],
            format!("{:#x}", proof.header.receipts_root)
        );
        assert_eq!(
            captured_proof["receipt_encoded"],
            format!("0x{}", alloy::hex::encode(proof.receipt.encoded_2718()))
        );
        let decode_nodes = |name: &str| -> Result<Vec<Vec<u8>>> {
            captured_proof[name]
                .as_array()
                .context("proof nodes")?
                .iter()
                .map(|node| Ok(alloy::hex::decode(node.as_str().context("proof node")?)?))
                .collect()
        };
        let captured = RecordedReceiptProof {
            index: captured_proof["index"].as_u64().context("proof index")?,
            raw_transaction: alloy::hex::decode(
                captured_proof["raw_transaction"]
                    .as_str()
                    .context("proof original")?,
            )?,
            transaction_proof: decode_nodes("transaction_proof")?,
            receipt_proof: decode_nodes("receipt_proof")?,
            ..proof.clone()
        };
        assert_eq!(captured.raw_transaction, proof.raw_transaction);
        validate_composition(&block, root, &captured)?;
        let archive = super::tests::Cleanup {
            directory: std::env::temp_dir().join(format!(
                "beacon-fixture-{}-{}",
                std::process::id(),
                header.slot
            )),
            genesis,
        };
        let path = archive.directory.join("receipt-proof.json");
        crate::finality_archive::atomic_json(&path, &proof)?;
        let retained: RecordedReceiptProof = serde_json::from_slice(&fs::read(&path)?)?;
        validate_composition(&block, root, &retained)?;
        assert_eq!(serde_json::to_vec(&proof)?, serde_json::to_vec(&retained)?);

        let mut changed_requests = block.clone();
        changed_requests["data"]["message"]["body"]["execution_requests"]["consolidations"] =
            serde_json::json!([]);
        assert!(validate_composition(&changed_requests, root, &proof).is_err());
        let mut changed_rlp = proof.clone();
        changed_rlp.header.requests_hash = Some(B256::ZERO);
        assert!(validate_composition(&block, root, &changed_rlp).is_err());
        let mut changed_payload = block.clone();
        changed_payload["data"]["message"]["body"]["execution_payload"]["block_hash"] =
            serde_json::json!(format!("{:#x}", B256::ZERO));
        assert!(validate_composition(&changed_payload, root, &proof).is_err());
        // Even a self-consistent changed Beacon root cannot replace the execution-header binding.
        let changed: beacon::electra::Block =
            serde::Deserialize::deserialize(&changed_payload["data"]["message"])?;
        assert!(validate_composition(
            &changed_payload,
            B256::from_slice(changed.tree_hash_root().as_bytes()),
            &proof
        )
        .is_err());
        let mut wrong_position = proof.clone();
        wrong_position.index += 1;
        assert!(validate_composition(&block, root, &wrong_position).is_err());
        assert!(verify_indexed_value(
            proof.header.receipts_root,
            wrong_position.index,
            &proof.receipt.encoded_2718(),
            &proof.receipt_proof
        )
        .is_err());
        let mut corrupted = retained;
        corrupted.receipt_proof[0][0] ^= 1;
        crate::finality_archive::atomic_json(&path, &corrupted)?;
        let corrupted: RecordedReceiptProof = serde_json::from_slice(&fs::read(path)?)?;
        assert!(validate_composition(&block, root, &corrupted).is_err());
    }
    Ok(())
}
