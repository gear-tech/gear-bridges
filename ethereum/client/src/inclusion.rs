//! Authenticated Ethereum execution blocks and retained transaction/receipt inclusion proofs.
use alloy::{
    consensus::TxEnvelope,
    eips::Encodable2718,
    primitives::{keccak256, B256},
    providers::Provider,
    rpc::types::BlockId,
};
use anyhow::{ensure, Context, Result};
use std::{fs, path::Path};

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedReceiptProof {
    pub genesis: B256,
    pub header: alloy::consensus::Header,
    pub index: u64,
    pub raw_transaction: Vec<u8>,
    pub receipt: alloy::consensus::ReceiptEnvelope,
    pub transaction_proof: Vec<Vec<u8>>,
    pub receipt_proof: Vec<Vec<u8>>,
}

pub fn verify_indexed_value(root: B256, index: u64, value: &[u8], proof: &[Vec<u8>]) -> Result<()> {
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

pub fn transaction_proof(
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
    pub fn verify_inclusion(
        &self,
        view: &crate::FinalizedView,
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
    pub fn verify_original_inclusion(
        &self,
        view: &crate::FinalizedView,
        raw: &[u8],
        hash: B256,
    ) -> Result<()> {
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
        Ok(())
    }
}

pub struct ReceiptBlockErrors {
    pub block: &'static str,
    pub receipts: &'static str,
    pub header: &'static str,
    pub transactions: &'static str,
    pub transaction_root: Option<&'static str>,
    pub count: &'static str,
    pub coordinates: &'static str,
    pub root: &'static str,
}

// Both receipt policies authenticate the complete block before selecting an original.
pub async fn authenticated_receipt_block(
    provider: &impl Provider,
    block_number: u64,
    block_hash: B256,
    errors: ReceiptBlockErrors,
) -> Result<(
    alloy::consensus::Header,
    Vec<TxEnvelope>,
    Vec<(u64, alloy::consensus::ReceiptEnvelope)>,
)> {
    let (block, receipts) = tokio::try_join!(
        async {
            provider
                .get_block_by_hash(block_hash)
                .full()
                .await?
                .context(errors.block)
        },
        async {
            provider
                .get_block_receipts(BlockId::hash_canonical(block_hash))
                .await?
                .context(errors.receipts)
        },
    )?;
    ensure!(
        block.header.number == block_number
            && block.header.hash == block_hash
            && block.header.inner.hash_slow() == block_hash,
        errors.header
    );
    let transactions = block
        .transactions
        .try_into_transactions()
        .map_err(|_| anyhow::anyhow!(errors.transactions))?
        .into_iter()
        .map(|tx| tx.into_inner())
        .collect::<Vec<_>>();
    if let Some(error) = errors.transaction_root {
        ensure!(
            alloy::consensus::proofs::calculate_transaction_root(&transactions)
                == block.header.transactions_root,
            error
        );
    }
    ensure!(receipts.len() == transactions.len(), errors.count);
    let mut envelopes = Vec::with_capacity(receipts.len());
    for (index, receipt) in receipts.into_iter().enumerate() {
        ensure!(
            receipt.transaction_index == Some(index as u64)
                && receipt.block_number == Some(block_number)
                && receipt.block_hash == Some(block_hash)
                && receipt.transaction_hash
                    == B256::from(keccak256(&transactions[index].encoded_2718())),
            errors.coordinates
        );
        envelopes.push((index as u64, receipt.into_primitives_receipt().inner));
    }
    ensure!(
        alloy::consensus::proofs::calculate_receipt_root(
            &envelopes.iter().map(|(_, receipt)| receipt).collect::<Vec<_>>()
        ) == block.header.receipts_root,
        errors.root
    );
    Ok((block.header.inner, transactions, envelopes))
}

/// Authenticate arbitrary signed transaction and receipt bytes; policy belongs to the caller.
pub async fn authenticate_transaction_receipt(
    provider: &impl Provider,
    view: &crate::FinalizedView,
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
    let (header, transactions, envelopes) = authenticated_receipt_block(
        provider,
        block_number,
        block_hash,
        ReceiptBlockErrors {
            block: "full inclusion block unavailable; HOLD",
            receipts: "inclusion receipts unavailable; HOLD",
            header: "inclusion header changed; HOLD",
            transactions: "full transactions missing; HOLD",
            transaction_root: None,
            count: "incomplete receipt set; HOLD",
            coordinates: "receipt coordinates changed; HOLD",
            root: "receipt trie differs from authenticated header; HOLD",
        },
    )
    .await?;
    let index = transactions
        .iter()
        .position(|tx| B256::from(keccak256(&tx.encoded_2718())) == hash)
        .context("signed transaction absent from inclusion; HOLD")? as u64;
    let receipt = ethereum_common::utils::generate_merkle_proof(index, &envelopes)?;
    let proof = RecordedReceiptProof {
        genesis: view.genesis_hash(),
        transaction_proof: transaction_proof(&transactions, index, header.transactions_root)?,
        header,
        index,
        raw_transaction: transactions[index as usize].encoded_2718(),
        receipt: receipt.receipt,
        receipt_proof: receipt.proof,
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

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        consensus::{
            proofs::{calculate_receipt_root, calculate_transaction_root},
            ReceiptEnvelope,
        },
        providers::ProviderBuilder,
        rpc::types::{Block, BlockTransactions, TransactionReceipt},
        transports::mock::Asserter,
    };

    #[tokio::test]
    async fn receipt_block_preserves_early_and_deferred_transaction_root_policy() -> Result<()> {
        for (early, bad_transactions, bad_receipts, expected) in [
            (true, false, false, None),
            (false, false, false, None),
            (true, true, true, Some("transaction root")),
            (false, true, true, Some("receipt root")),
            (true, false, true, Some("receipt root")),
            (false, true, false, None),
        ] {
            let mut block: Block = Default::default();
            block.transactions = BlockTransactions::Full(Vec::new());
            block.header.inner.transactions_root = if bad_transactions {
                B256::ZERO
            } else {
                calculate_transaction_root(&[] as &[TxEnvelope])
            };
            block.header.inner.receipts_root = if bad_receipts {
                B256::ZERO
            } else {
                calculate_receipt_root(&[] as &[ReceiptEnvelope])
            };
            block.header.hash = block.header.inner.hash_slow();
            let asserter = Asserter::new();
            let provider = ProviderBuilder::new().connect_mocked_client(asserter.clone());
            asserter.push_success(&block);
            asserter.push_success(&Vec::<TransactionReceipt>::new());
            let result = authenticated_receipt_block(
                &provider,
                0,
                block.header.hash,
                ReceiptBlockErrors {
                    block: "block",
                    receipts: "receipts",
                    header: "header",
                    transactions: "transactions",
                    transaction_root: early.then_some("transaction root"),
                    count: "count",
                    coordinates: "coordinates",
                    root: "receipt root",
                },
            )
            .await;
            assert_eq!(result.err().map(|error| error.to_string()).as_deref(), expected);
        }
        Ok(())
    }
}
