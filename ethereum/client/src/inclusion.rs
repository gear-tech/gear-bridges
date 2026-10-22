//! Authenticated Ethereum execution blocks and retained transaction, receipt and account proofs.
use alloy::{
    consensus::{Header, TrieAccount, TxEnvelope},
    eips::Encodable2718,
    primitives::{keccak256, Address, B256},
    providers::Provider,
    rlp::Decodable,
    rpc::types::BlockId,
};
use anyhow::{ensure, Context, Result};
use std::{fs, path::Path};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
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

/// Account evidence at an exact authenticated header, including proven account absence.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordedAccountProof {
    pub genesis: B256,
    pub header: Header,
    pub address: Address,
    pub account_proof: Vec<Vec<u8>>,
}

impl RecordedAccountProof {
    pub fn verify(
        &self,
        view: &crate::FinalizedView,
        expected_address: Address,
    ) -> Result<Option<TrieAccount>> {
        use ethereum_common::{
            hash_db, memory_db,
            patricia_trie::TrieDB,
            trie_db::{HashDB, Trie},
        };
        ensure!(
            self.genesis == view.genesis_hash()
                && view.contains(self.header.number, self.header.hash_slow()),
            "account proof is not bound to authenticated finalized history; HOLD"
        );
        ensure!(
            self.address == expected_address,
            "account proof address differs from the original signer; HOLD"
        );
        let mut database = memory_db::new();
        for node in &self.account_proof {
            database.insert(hash_db::EMPTY_PREFIX, node);
        }
        let root = ethereum_common::H256(self.header.state_root.0);
        let trie = TrieDB::new(&database, &root)
            .map_err(|_| anyhow::anyhow!("invalid account proof database; HOLD"))?;
        let value = trie
            .get(keccak256(self.address).as_slice())
            .map_err(|_| anyhow::anyhow!("unavailable account proof node; HOLD"))?;
        let Some(value) = value else {
            return Ok(None);
        };
        let mut remaining = value.as_slice();
        let account = TrieAccount::decode(&mut remaining)
            .context("malformed account proof RLP; HOLD")?;
        ensure!(
            remaining.is_empty() && alloy::rlp::encode(account) == value,
            "noncanonical account proof RLP; HOLD"
        );
        Ok(Some(account))
    }
}

/// Acquire account evidence without trusting the RPC's account-field claims.
pub async fn prove_account(
    provider: &impl Provider,
    view: &crate::FinalizedView,
    header: Header,
    address: Address,
) -> Result<RecordedAccountProof> {
    let hash = header.hash_slow();
    ensure!(
        view.contains(header.number, hash),
        "account proof block is not authenticated; HOLD"
    );
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        provider
            .get_proof(address, Vec::new())
            .block_id(BlockId::hash_canonical(hash))
            .await
    })
    .await
    .map_err(alloy::transports::TransportErrorKind::custom)
    .context("account proof request timed out; HOLD")?
    .context("account proof request failed; HOLD")?;
    ensure!(
        response.address == address && response.storage_proof.is_empty(),
        "account proof response changed the requested address or storage keys; HOLD"
    );
    let proof = RecordedAccountProof {
        genesis: view.genesis_hash(),
        header,
        address,
        account_proof: response
            .account_proof
            .into_iter()
            .map(|node| node.to_vec())
            .collect(),
    };
    // Only authenticated absence permits the canonical empty-account values.
    let account = proof.verify(view, address)?.unwrap_or_default();
    ensure!(
        response.nonce == account.nonce
            && response.balance == account.balance
            && response.storage_hash == account.storage_root
            && response.code_hash == account.code_hash,
        "account proof response disagrees with the authenticated account; HOLD"
    );
    Ok(proof)
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
    pub fn verify_inclusion(&self, view: &crate::FinalizedView, hash: B256) -> Result<()> {
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
        ensure!(
            self.raw_transaction == raw,
            "original receipt proof changed signed transaction identity; HOLD"
        );
        self.verify_inclusion(view, hash)
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
            tokio::time::timeout(std::time::Duration::from_secs(10),
                provider.get_block_by_hash(block_hash).full())
                .await.map_err(alloy::transports::TransportErrorKind::custom)??
                .ok_or(crate::Error::FinalizedEvidenceUnavailable("full block"))
                .context(errors.block)
        },
        async {
            tokio::time::timeout(std::time::Duration::from_secs(10),
                provider.get_block_receipts(BlockId::hash_canonical(block_hash)))
                .await.map_err(alloy::transports::TransportErrorKind::custom)??
                .ok_or(crate::Error::FinalizedEvidenceUnavailable("block receipts"))
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
        .map_err(|_| crate::Error::FinalizedEvidenceUnavailable("full transactions"))
        .context(errors.transactions)?
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
    if receipts.len() < transactions.len() {
        return Err(crate::Error::FinalizedEvidenceUnavailable("incomplete receipt set")).context(errors.count);
    }
    ensure!(receipts.len() == transactions.len(), errors.count);
    let mut envelopes = Vec::with_capacity(receipts.len());
    let mut global_log_index = 0u64;
    for (index, receipt) in receipts.into_iter().enumerate() {
        ensure!(
            receipt.transaction_index == Some(index as u64)
                && receipt.block_number == Some(block_number)
                && receipt.block_hash == Some(block_hash)
                && receipt.transaction_hash
                    == B256::from(keccak256(transactions[index].encoded_2718())),
            errors.coordinates
        );
        for log in receipt.inner.logs() {
            ensure!(
                !log.removed
                    && log.block_number == Some(block_number)
                    && log.block_hash == Some(block_hash)
                    && log.transaction_hash == Some(receipt.transaction_hash)
                    && log.transaction_index == Some(index as u64)
                    && log.log_index == Some(global_log_index),
                errors.coordinates
            );
            global_log_index = global_log_index
                .checked_add(1)
                .context(errors.coordinates)?;
        }
        envelopes.push((index as u64, receipt.into_primitives_receipt().inner));
    }
    ensure!(
        alloy::consensus::proofs::calculate_receipt_root(
            &envelopes
                .iter()
                .map(|(_, receipt)| receipt)
                .collect::<Vec<_>>()
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
        view.recheck(provider).await?;
        return Ok(proof);
    }
    let proof = prove_transaction_receipt(provider, view, block_number, block_hash, hash).await?;
    crate::finality_archive::atomic_json(&path, &proof)?;
    Ok(proof)
}

/// Build retained inclusion evidence without creating or touching a cache directory.
pub async fn prove_transaction_receipt(
    provider: &impl Provider,
    view: &crate::FinalizedView,
    block_number: u64,
    block_hash: B256,
    hash: B256,
) -> Result<RecordedReceiptProof> {
    Ok(
        prove_transaction_receipt_with_log_index(provider, view, block_number, block_hash, hash)
            .await?
            .0,
    )
}

pub(crate) async fn prove_transaction_receipt_with_log_index(
    provider: &impl Provider,
    view: &crate::FinalizedView,
    block_number: u64,
    block_hash: B256,
    hash: B256,
) -> Result<(RecordedReceiptProof, u64)> {
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
        .position(|tx| B256::from(keccak256(tx.encoded_2718())) == hash)
        .context("signed transaction absent from inclusion; HOLD")? as u64;
    let first_log_index =
        envelopes
            .iter()
            .take(index as usize)
            .try_fold(0u64, |total, (_, receipt)| {
                total
                    .checked_add(u64::try_from(receipt.logs().len())?)
                    .context("receipt log index overflow")
            })?;
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
    view.recheck(provider).await?;
    Ok((proof, first_log_index))
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

    fn account_fixture(
        value: Option<&[u8]>,
        sibling: bool,
    ) -> Result<(RecordedAccountProof, crate::FinalizedView)> {
        use ethereum_common::{
            memory_db,
            patricia_trie::{TrieDB, TrieDBMut},
            trie_db::{Recorder, Trie, TrieMut},
        };
        let address = Address::repeat_byte(41);
        let key = keccak256(address);
        let mut database = memory_db::new();
        let mut root = ethereum_common::H256::zero();
        {
            let mut trie = TrieDBMut::new(&mut database, &mut root);
            if let Some(value) = value {
                trie.insert(key.as_slice(), value).unwrap();
            }
            if sibling {
                trie.insert(
                    keccak256(Address::repeat_byte(42)).as_slice(),
                    &alloy::rlp::encode(TrieAccount::default()),
                )
                .unwrap();
            }
            let _ = trie.root();
        }
        let trie = TrieDB::new(&database, &root).unwrap();
        let mut recorder = Recorder::new();
        assert_eq!(
            trie.get_with(key.as_slice(), &mut recorder).unwrap().as_deref(),
            value
        );
        let header = Header {
            number: 17,
            state_root: B256::from(root.0),
            ..Default::default()
        };
        // These are genuine signed LocalFixture pins; the tests below exercise
        // account-trie failures independently of finality acquisition.
        let execution = alloy::rpc::types::Header {
            hash: header.hash_slow(), inner: header.clone(), ..Default::default()
        };
        let view = std::thread::Builder::new().stack_size(32 * 1024 * 1024).spawn(move || {
            tokio::runtime::Runtime::new()?.block_on(async move {
                let mut genesis: Block = Block::default();
                genesis.header.inner.extra_data = execution.state_root.as_slice().to_vec().into();
                genesis.header.hash = genesis.header.inner.hash_slow();
                let head = execution.clone();
                let fixture = crate::beacon_finality::test_fixtures::LocalBeaconFixture::new(
                    31337, genesis.header.hash, std::sync::Arc::new(move || head.clone()),
                ).await?;
                let authority = crate::beacon_finality::BeaconAuthority::open(fixture.config.clone(), &fixture.directory).await?;
                let asserter = Asserter::new();
                let provider = alloy::providers::RootProvider::builder().connect_mocked_client(asserter.clone());
                asserter.push_success(&alloy::primitives::U64::from(31337));
                asserter.push_success(&genesis);
                let head: Block = Block { header: execution, ..Default::default() };
                asserter.push_success(&head);
                asserter.push_success(&head);
                crate::verified_finalized_view(&provider, &authority, None).await.map_err(anyhow::Error::from)
            })
        })?.join().map_err(|_| anyhow::anyhow!("signed account fixture thread panicked"))??;
        let genesis = view.genesis_hash();
        Ok((
            RecordedAccountProof {
                genesis,
                header,
                address,
                account_proof: recorder.drain().into_iter().map(|node| node.data).collect(),
            },
            view,
        ))
    }

    #[test]
    fn account_presence_absence_and_retained_identity() -> Result<()> {
        let account = TrieAccount {
            nonce: 19,
            balance: alloy::primitives::U256::from(37),
            ..Default::default()
        };
        let encoded = alloy::rlp::encode(account);
        let (proof, view) = account_fixture(Some(&encoded), true)?;
        assert_eq!(proof.verify(&view, proof.address)?, Some(account));
        let saved = serde_json::to_vec(&proof)?;
        let restored: RecordedAccountProof = serde_json::from_slice(&saved)?;
        assert_eq!(restored.verify(&view, proof.address)?, Some(account));
        let mut unknown = serde_json::to_value(&proof)?;
        unknown["nonce"] = serde_json::json!(0);
        assert!(serde_json::from_value::<RecordedAccountProof>(unknown).is_err());
        assert!(proof.verify(&view, Address::repeat_byte(42)).is_err());
        for field in 0..5 {
            let mut changed = proof.clone();
            match field {
                0 => changed.genesis = B256::ZERO,
                1 => changed.header.number += 1,
                2 => changed.header.state_root = B256::ZERO,
                3 => changed.header.timestamp += 1,
                _ => changed.address = Address::repeat_byte(42),
            }
            assert!(changed.verify(&view, proof.address).is_err());
        }
        // Removing a required child must not turn a present account into nonce zero.
        assert!(proof.account_proof.len() > 1);
        let mut incomplete = proof.clone();
        incomplete.account_proof.pop();
        assert!(incomplete.verify(&view, proof.address).is_err());
        incomplete.account_proof.clear();
        assert!(incomplete.verify(&view, proof.address).is_err());
        for sibling in [false, true] {
            let (absent, view) = account_fixture(None, sibling)?;
            assert_eq!(absent.verify(&view, absent.address)?, None);
            if sibling {
                let mut incomplete = absent.clone();
                incomplete.account_proof.clear();
                assert!(incomplete.verify(&view, absent.address).is_err());
            }
        }
        Ok(())
    }

    #[test]
    fn account_leaf_requires_exact_canonical_rlp() -> Result<()> {
        let canonical = alloy::rlp::encode(TrieAccount::default());
        let mut trailing = canonical.clone();
        trailing.push(0);
        let mut noncanonical = canonical.clone();
        // The zero nonce starts after the long-list prefix and payload length.
        assert_eq!(noncanonical[2], 0x80);
        noncanonical[2] = 0;
        let mut truncated = canonical.clone();
        truncated.pop();
        for value in [vec![0xc0], trailing, noncanonical, truncated] {
            let (proof, view) = account_fixture(Some(&value), false)?;
            assert!(proof.verify(&view, proof.address).is_err());
        }
        Ok(())
    }

    fn account_response(
        proof: &RecordedAccountProof,
        account: TrieAccount,
    ) -> alloy::rpc::types::EIP1186AccountProofResponse {
        alloy::rpc::types::EIP1186AccountProofResponse {
            address: proof.address,
            balance: account.balance,
            code_hash: account.code_hash,
            nonce: account.nonce,
            storage_hash: account.storage_root,
            account_proof: proof.account_proof.iter().cloned().map(Into::into).collect(),
            storage_proof: Vec::new(),
        }
    }

    #[tokio::test]
    async fn account_rpc_claims_and_exact_hash_coordinates() -> Result<()> {
        for present in [true, false] {
            let account = TrieAccount {
                nonce: if present { 7 } else { 0 },
                ..Default::default()
            };
            let encoded = alloy::rlp::encode(account);
            let (proof, view) = account_fixture(present.then_some(encoded.as_slice()), true)?;
            for mutation in 0..7 {
                let mut response = account_response(&proof, account);
                match mutation {
                    1 => response.nonce += 1,
                    2 => response.balance += alloy::primitives::U256::from(1),
                    3 => response.storage_hash = B256::ZERO,
                    4 => response.code_hash = B256::ZERO,
                    5 => response.address = Address::ZERO,
                    6 => response.account_proof.clear(),
                    _ => {}
                }
                let asserter = Asserter::new();
                asserter.push_success(&response);
                let expected = serde_json::json!([
                    proof.address,
                    [],
                    {"blockHash": proof.header.hash_slow(), "requireCanonical": true}
                ]);
                let client = alloy::rpc::client::RpcClient::builder()
                    .layer(tower::util::MapRequestLayer::new(
                        move |request: alloy::rpc::json_rpc::RequestPacket| {
                            let json = serde_json::to_value(&request).unwrap();
                            assert_eq!(json["method"], "eth_getProof");
                            assert_eq!(json["params"], expected);
                            request
                        },
                    ))
                    .transport(
                        alloy::transports::mock::MockTransport::new(asserter.clone()),
                        true,
                    );
                let provider = alloy::providers::RootProvider::builder().connect_client(client);
                let result = prove_account(&provider, &view, proof.header.clone(), proof.address).await;
                if mutation == 0 {
                    assert_eq!(
                        result?.verify(&view, proof.address)?,
                        present.then_some(account)
                    );
                } else {
                    assert!(result.is_err(), "accepted RPC mutation {mutation}");
                }
                assert!(asserter.pop_response().is_none());
                asserter.push_success(&response);
                let mut wrong_header = proof.header.clone();
                wrong_header.number += 1;
                assert!(prove_account(&provider, &view, wrong_header, proof.address).await.is_err());
                assert!(asserter.pop_response().is_some(), "unauthenticated header reached RPC");
            }
        }
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn account_rpc_has_an_individual_deadline() -> Result<()> {
        let (proof, view) = account_fixture(None, false)?;
        let asserter = Asserter::new();
        asserter.push_success(&account_response(&proof, TrieAccount::default()));
        let client = alloy::rpc::client::RpcClient::builder()
            .layer(tower::util::MapFutureLayer::new(
                |future| -> alloy::transports::TransportFut<'static> {
                    Box::pin(async move {
                        tokio::time::sleep(std::time::Duration::from_secs(11)).await;
                        future.await
                    })
                },
            ))
            .transport(alloy::transports::mock::MockTransport::new(asserter), true);
        let provider = alloy::providers::RootProvider::builder().connect_client(client);
        let error = prove_account(&provider, &view, proof.header, proof.address)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        assert!(crate::error::is_transient_rpc_error(&error));
        Ok(())
    }

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
            let mut block: Block = Block {
                transactions: BlockTransactions::Full(Vec::new()),
                ..Default::default()
            };
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
            assert_eq!(
                result.err().map(|error| error.to_string()).as_deref(),
                expected
            );
        }
        Ok(())
    }
}
