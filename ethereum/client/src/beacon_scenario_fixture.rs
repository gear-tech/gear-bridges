//! Offline, test-only scenario export. All keys and trust parameters are public LocalFixture data.
//! Input coordinates are decimal integers; output blocks/receipts use normal Ethereum RPC quantities.
use super::*;
use alloy::{
    consensus::{
        proofs::{calculate_receipt_root, calculate_transaction_root},
        Receipt, ReceiptEnvelope, SignableTransaction, TxEip1559, TxEnvelope,
    },
    eips::Encodable2718,
    network::TxSigner,
    primitives::{Address, Log, TxKind},
    rpc::types::{Block, BlockTransactions, TransactionReceipt},
    signers::local::PrivateKeySigner,
};
use std::collections::BTreeSet;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Scenario {
    receipts: Vec<ScenarioReceipt>,
    finalized_height: u64,
    #[serde(default)]
    finalized_heights: Vec<u64>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ScenarioReceipt {
    block_number: u64,
    transaction_index: u64,
    logs: Vec<ScenarioLog>,
    #[serde(default = "successful")]
    status: bool,
}
fn successful() -> bool {
    true
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ScenarioLog {
    address: Address,
    topics: Vec<B256>,
    data: Bytes,
}

/// Write an immutable signed LocalFixture bundle without contacting or starting any RPC endpoint.
/// `finalizedHeights` selects additional finality responses; `finalizedHeight` remains the default.
/// At most 10,000 headers, 1,024 receipts/selected heights and 16 MiB of input are accepted.
/// Receipt indices must be contiguous from zero in each block (including explicit empty placeholders).
pub async fn export_scenario(input: &[u8], output: impl AsRef<Path>) -> Result<B256> {
    let bundle = scenario_bundle(input).await?;
    let hash = digest(&serde_json::to_vec(&bundle)?);
    gear_common::durable::immutable_json(output.as_ref(), &bundle)?;
    Ok(hash)
}

async fn scenario_bundle(input: &[u8]) -> Result<Value> {
    ensure!(
        input.len() <= 16 * 1024 * 1024,
        "scenario input exceeds 16 MiB"
    );
    let mut spec: Scenario = serde_json::from_slice(input)?;
    ensure!(
        spec.receipts.len() <= 1024 && spec.finalized_heights.len() <= 1024,
        "scenario exceeds receipt or selected-height limit"
    );
    let mut heights: BTreeSet<u64> = spec.finalized_heights.into_iter().collect();
    heights.insert(spec.finalized_height);
    heights.insert(0);
    ensure!(
        heights.len() <= 1024,
        "scenario exceeds selected-height limit"
    );
    let maximum = heights
        .iter()
        .copied()
        .chain(spec.receipts.iter().map(|r| r.block_number))
        .max()
        .unwrap();
    ensure!(maximum < 10_000, "scenario exceeds 10,000 headers");
    spec.receipts
        .sort_by_key(|r| (r.block_number, r.transaction_index));
    let mut expected = 0;
    let mut previous = None;
    for receipt in &spec.receipts {
        if previous != Some(receipt.block_number) {
            expected = 0;
        }
        ensure!(
            receipt.transaction_index == expected,
            "scenario transaction indices must be contiguous from zero"
        );
        ensure!(
            receipt.logs.len() <= 1024,
            "scenario receipt exceeds log limit"
        );
        for log in &receipt.logs {
            ensure!(
                log.topics.len() <= 4 && log.data.len() <= 65_536,
                "scenario log exceeds topic/data limit"
            );
        }
        expected += 1;
        previous = Some(receipt.block_number);
    }

    let signer = PrivateKeySigner::from_bytes(&B256::repeat_byte(62))?;
    let recipient = Address::repeat_byte(9);
    let mut blocks = Vec::with_capacity(maximum as usize + 1);
    let mut receipts: Vec<TransactionReceipt> = Vec::with_capacity(spec.receipts.len());
    let mut raw_by_height = BTreeMap::new();
    let mut source = spec.receipts.into_iter().peekable();
    let mut parent = B256::ZERO;
    for number in 0..=maximum {
        let mut transactions = Vec::new();
        let mut consensus_receipts = Vec::new();
        while source
            .peek()
            .is_some_and(|receipt| receipt.block_number == number)
        {
            let receipt = source.next().unwrap();
            let mut transaction = TxEip1559 {
                chain_id: 31337,
                // Coordinates, not scenario contents, determine the signed identity.
                nonce: number * 1024 + receipt.transaction_index,
                gas_limit: 100_000,
                max_fee_per_gas: 10,
                max_priority_fee_per_gas: 1,
                to: TxKind::Call(recipient),
                ..Default::default()
            };
            let signature = signer.sign_transaction(&mut transaction).await?;
            let envelope: TxEnvelope = transaction.into_signed(signature).into();
            transactions.push(envelope);
            let logs = receipt
                .logs
                .into_iter()
                .map(|log| Ok(serde_json::from_value::<Log>(serde_json::to_value(log)?)?))
                .collect::<Result<Vec<_>>>()?;
            consensus_receipts.push(ReceiptEnvelope::Eip1559(
                Receipt {
                    status: receipt.status.into(),
                    cumulative_gas_used: transactions.len() as u64 * 21_000,
                    logs,
                }
                .with_bloom(),
            ));
        }
        let mut block: Block = Block::default();
        block.header.inner.number = number;
        block.header.inner.parent_hash = parent;
        block.header.inner.timestamp = 1_700_000_000 + number;
        block.header.inner.extra_data = Bytes::from_static(b"LocalFixture chain31337");
        block.header.inner.gas_limit = 100_000_000;
        block.header.inner.gas_used = transactions.len() as u64 * 21_000;
        block.header.inner.base_fee_per_gas = Some(1);
        block.header.inner.transactions_root = calculate_transaction_root(&transactions);
        block.header.inner.receipts_root = calculate_receipt_root(&consensus_receipts);
        for receipt in &consensus_receipts {
            for log in receipt.logs() {
                block.header.inner.logs_bloom.accrue_log(log);
            }
        }
        block.header.hash = block.header.inner.hash_slow();
        parent = block.header.hash;
        let mut rpc_transactions = Vec::with_capacity(transactions.len());
        let mut raw = Vec::with_capacity(transactions.len());
        let mut global_log_index = 0u64;
        for (index, (transaction, receipt)) in
            transactions.into_iter().zip(consensus_receipts).enumerate()
        {
            let hash = *transaction.tx_hash();
            raw.push(hex(transaction.encoded_2718()));
            let coordinates = json!({"transactionHash":hash, "transactionIndex":format!("0x{index:x}"),
                "blockNumber":format!("0x{number:x}"), "blockHash":block.header.hash});
            let mut value = serde_json::to_value(receipt)?;
            value
                .as_object_mut()
                .unwrap()
                .extend(coordinates.as_object().unwrap().clone());
            value.as_object_mut().unwrap().extend(
                json!({"gasUsed":"0x5208", "effectiveGasPrice":"0x2",
                "from":signer.address(), "to":recipient, "contractAddress":null})
                .as_object()
                .unwrap()
                .clone(),
            );
            for log in value["logs"].as_array_mut().unwrap() {
                log.as_object_mut()
                    .unwrap()
                    .extend(coordinates.as_object().unwrap().clone());
                log["logIndex"] = json!(format!("0x{global_log_index:x}"));
                log["removed"] = json!(false);
                global_log_index += 1;
            }
            receipts.push(serde_json::from_value(value)?);
            let mut value = serde_json::to_value(transaction)?;
            value
                .as_object_mut()
                .unwrap()
                .extend(coordinates.as_object().unwrap().clone());
            value.as_object_mut().unwrap().remove("transactionHash");
            value["hash"] = json!(hash);
            value["from"] = json!(signer.address());
            rpc_transactions.push(serde_json::from_value(value)?);
        }
        block.transactions = BlockTransactions::Full(rpc_transactions);
        if heights.contains(&number) {
            raw_by_height.insert(number, raw);
        }
        blocks.push(block);
    }

    let genesis = blocks[0].header.hash;
    let mut bundle = execution_bundle(&blocks, &receipts, genesis)?;
    let network = Network::LocalFixture {
        genesis_time: 1_700_000_000,
        genesis_validators_root: [101; 32].into(),
    };
    let mut generator = Generator::new(network.clone());
    let initial = generator.signed_payload(&blocks[0].header, 8192, &raw_by_height[&0]);
    let approval = BootstrapApproval {
        schema_version: 1,
        chain_id: 31337,
        genesis_hash: genesis,
        genesis_validators_root: B256::from(network.genesis_validators_root().0),
        genesis_time: network.genesis_time(),
        network,
        trusted_bootstrap_root: B256::from(initial.header.tree_hash_root().0),
        trusted_bootstrap_slot: 8192,
        approved_at_ms: 1_700_000_000_000,
        bootstrap_expires_at_ms: 2_000_000_000_000,
    };
    let approval_bytes = serde_json::to_vec(&approval)?;
    let mut finality = BTreeMap::new();
    finality.insert("0".to_owned(), initial.update.clone());
    for (ordinal, height) in heights
        .into_iter()
        .filter(|height| *height != 0)
        .enumerate()
    {
        // Execution height is independent of Beacon slot. Keep all selected updates in one period.
        let signed = generator.signed_payload(
            &blocks[height as usize].header,
            8192 + (ordinal as u64 + 1) * 4,
            &raw_by_height[&height],
        );
        finality.insert(height.to_string(), signed.update);
    }
    let beacon_blocks = generator
        .blocks
        .iter()
        .map(|(root, block)| {
            (
                hex(root),
                json!({"version":"electra", "execution_optimistic":false, "finalized":true,
            "data":{"message":block}}),
            )
        })
        .collect::<BTreeMap<_, _>>();
    bundle.as_object_mut().unwrap().extend(json!({
        "schemaVersion":1, "description":"Deterministic signed LocalFixture chain31337; NOT public-network proof or production approval",
        "approvalFileBytes":Bytes::from(approval_bytes.clone()), "approvalSha256":digest(&approval_bytes),
        "bootstrap":initial.bootstrap, "updates":[initial.update],
        "finalityUpdate":finality[&spec.finalized_height.to_string()],
        "finalityUpdatesByExecutionNumber":finality, "beaconBlocksByRoot":beacon_blocks,
    }).as_object().unwrap().clone());
    Ok(bundle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_export_has_canonical_roots_and_selectable_signed_heights() -> Result<()> {
        // Consensus SSZ containers require the same large stack as the existing Beacon tests.
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?
                    .block_on(async {
                        let spec = json!({"receipts":(0..4).map(|index| json!({"blockNumber":11,
                    "transactionIndex":index,"logs":[{"address":Address::repeat_byte(7),
                        "topics":[B256::repeat_byte(8)],"data":"0x0102"}]})).collect::<Vec<_>>(),
                    "finalizedHeight":11,"finalizedHeights":[11,12,13]});
                        let input = serde_json::to_vec(&spec)?;
                        let bundle = scenario_bundle(&input).await?;
                        assert_eq!(bundle, scenario_bundle(&input).await?);
                        let blocks: Vec<Block> =
                            serde::Deserialize::deserialize(&bundle["executionBlocks"])?;
                        let receipts: Vec<TransactionReceipt> =
                            serde::Deserialize::deserialize(&bundle["receipts"])?;
                        assert_eq!(blocks.len(), 14);
                        assert_eq!(receipts.len(), 4);
                        assert_eq!(bundle["inclusionProofs"].as_array().unwrap().len(), 4);
                        for block in &blocks {
                            assert_eq!(block.header.hash, block.header.inner.hash_slow());
                        }
                        for (index, receipt) in receipts.iter().enumerate() {
                            assert_eq!(receipt.transaction_index, Some(index as u64));
                            assert_eq!(receipt.inner.logs()[0].log_index, Some(index as u64));
                        }
                        let committee: beacon::SyncCommittee = serde::Deserialize::deserialize(
                            &bundle["bootstrap"]["data"]["current_sync_committee"],
                        )?;
                        let committee = keys(&committee)?;
                        let network = Network::LocalFixture {
                            genesis_time: 1_700_000_000,
                            genesis_validators_root: [101; 32].into(),
                        };
                        for height in [11usize, 12, 13] {
                            let wire: utils::Update = serde::Deserialize::deserialize(
                                &bundle["finalityUpdatesByExecutionNumber"][height.to_string()]
                                    ["data"],
                            )?;
                            let root = wire.finalized_header.tree_hash_root();
                            let block: beacon::electra::Block = serde::Deserialize::deserialize(
                                &bundle["beaconBlocksByRoot"][hex(root)]["data"]["message"],
                            )?;
                            assert_eq!(block.tree_hash_root(), root);
                            assert_eq!(
                                block.body.execution_payload.block_hash.as_ref(),
                                blocks[height].header.hash.as_slice()
                            );
                            let (update, aggregate) = update_wire(wire)?;
                            let verified = verification::verify_update(
                                VerificationState {
                                    network: &network,
                                    finalized_slot: 8192,
                                    current_committee: &committee,
                                    next_committee: &committee,
                                },
                                1_700_200_000_000,
                                update,
                                &aggregate,
                                &NativeBls,
                            )
                            .await?;
                            assert!(verified.finalized_header.is_some());
                        }
                        let mut bad = spec.clone();
                        bad["unknown"] = json!(true);
                        assert!(scenario_bundle(&serde_json::to_vec(&bad)?).await.is_err());
                        let mut bad = spec;
                        bad["receipts"][1]["transactionIndex"] = json!(3);
                        assert!(scenario_bundle(&serde_json::to_vec(&bad)?).await.is_err());
                        Ok(())
                    })
            })?
            .join()
            .unwrap()
    }
}
