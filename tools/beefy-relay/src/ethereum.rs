use alloy::{
    node_bindings::{Anvil, AnvilInstance},
    primitives::{Address, Bytes, FixedBytes, B256, U256},
    providers::Provider,
    sol,
};
use anyhow::{bail, ensure, Context, Result};
use ethereum_client::{
    abi::IMessageQueue::IMessageQueueErrors, Error as EthereumClientError, EthApi,
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
};
use tokio::process::Command;

use beefy_relay::{
    authority_proof, keccak256, Hash32, RuntimeLeaf, SimplifiedMmrProof, ValidatedCommitment,
};

sol!(
    #![sol(rpc, extra_derives(Debug))]
    BeefyClient,
    "../../api/ethereum/BeefyClient.json"
);

sol!(
    #![sol(rpc, extra_derives(Debug))]
    VaraQueueRootVerifier,
    "../../api/ethereum/VaraQueueRootVerifier.json"
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DestinationCheckpoint {
    pub block: u64,
    pub root: Hash32,
    pub current_id: u64,
    pub current_root: Hash32,
    pub next_id: u64,
    pub next_root: Hash32,
}

pub struct Ethereum {
    _anvil: AnvilInstance,
    pub api: EthApi,
    pub client_address: Address,
    pub verifier_address: Address,
    pub queue_address: Address,
    pub receiver_address: Address,
    pub transactions: Vec<Value>,
}

impl Ethereum {
    pub async fn prepare(output: &Path, genesis_root: Hash32) -> Result<Self> {
        let anvil = Anvil::new()
            .host("127.0.0.1")
            .port(0u16)
            .chain_id(31_337)
            .try_spawn()
            .context("spawn loopback Anvil")?;
        let private_key = format!("0x{}", hex::encode(anvil.first_key().to_bytes()));
        let authority_root = format!("0x{}", hex::encode(genesis_root));
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let endpoint = anvil.endpoint();
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
            .env("PRIVATE_KEY", &private_key)
            .env("BEEFY_AUTHORITY_ROOT", &authority_root)
            .stdin(Stdio::null())
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

        let broadcast = root.join("ethereum/broadcast/BeefyLocal.s.sol/31337/run-latest.json");
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
            _anvil: anvil,
            api,
            client_address,
            verifier_address,
            queue_address,
            receiver_address,
            transactions,
        };
        let checkpoint = ethereum.checkpoint().await?;
        ensure!(
            checkpoint.block == 0,
            "new BeefyClient has nonzero BEEFY block"
        );
        ensure!(
            checkpoint.root == [0; 32],
            "new BeefyClient has a nonzero MMR root"
        );
        ensure!(
            checkpoint.current_id == 0
                && checkpoint.current_root == genesis_root
                && checkpoint.next_id == 1
                && checkpoint.next_root == genesis_root,
            "deployment authority checkpoint does not match trusted local genesis"
        );
        Ok(ethereum)
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
        Ok(json!({
            "chainId": 31337,
            "client": address_string(self.client_address),
            "verifier": address_string(self.verifier_address),
            "queue": address_string(self.queue_address),
            "receiver": address_string(self.receiver_address),
            "bytecodeHashes": bytecode_hashes,
        }))
    }

    pub async fn checkpoint(&self) -> Result<DestinationCheckpoint> {
        let client = BeefyClient::new(self.client_address, self.api.raw_provider().clone());
        let current = client
            .currentValidatorSet()
            .call()
            .await
            .context("read current BEEFY set")?;
        let next = client
            .nextValidatorSet()
            .call()
            .await
            .context("read next BEEFY set")?;
        Ok(DestinationCheckpoint {
            block: client
                .latestBeefyBlock()
                .call()
                .await
                .context("read latest BEEFY block")?,
            root: client
                .latestMMRRoot()
                .call()
                .await
                .context("read latest MMR root")?
                .into(),
            current_id: u64::try_from(current.id).context("current BEEFY set id overflows u64")?,
            current_root: current.root.into(),
            next_id: u64::try_from(next.id).context("next BEEFY set id overflows u64")?,
            next_root: next.root.into(),
        })
    }

    pub async fn submit_commitment(
        &mut self,
        validated: &ValidatedCommitment,
        keys: &[Vec<u8>],
        handover_leaf: &RuntimeLeaf,
        handover: &SimplifiedMmrProof,
    ) -> Result<()> {
        let before = self.checkpoint().await?;
        let set_id = validated.signed.commitment.validator_set_id;
        ensure!(
            set_id == before.current_id || set_id == before.next_id,
            "commitment authority set {set_id} is not current or next"
        );
        if set_id == before.next_id {
            ensure!(
                u64::from(handover_leaf.beefy_next_authority_set.id) > before.next_id,
                "next-set commitment does not carry a later authority set"
            );
        }

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

        let leaf = to_sol_leaf(handover_leaf)?;
        let items: Vec<B256> = handover.items.iter().copied().map(B256::from).collect();
        let order = U256::from_be_bytes(handover.proof_order);
        let receipt = client
            .submitFiatShamir(commitment, bitfield, proofs, leaf, items, order)
            .send()
            .await
            .context("submit Fiat-Shamir commitment")?
            .get_receipt()
            .await
            .context("wait for Fiat-Shamir commitment receipt")?;
        ensure!(
            receipt.status(),
            "Fiat-Shamir commitment transaction reverted"
        );
        self.record_receipt("submitFiatShamir", &receipt, None);

        let after = self.checkpoint().await?;
        ensure!(
            after.block == u64::from(validated.signed.commitment.block_number)
                && after.root == validated.mmr_root,
            "accepted BEEFY checkpoint does not match commitment"
        );
        if set_id == before.next_id {
            ensure!(
                after.current_id == before.next_id
                    && after.next_id == u64::from(handover_leaf.beefy_next_authority_set.id)
                    && after.next_root == handover_leaf.beefy_next_authority_set.keyset_commitment,
                "accepted next-set transition does not match handover leaf"
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

    pub async fn register(&mut self, source: u32, root: Hash32, encoded: Vec<u8>) -> Result<()> {
        let pending = self
            .api
            .provide_merkle_root(source, root, encoded)
            .await
            .context("submit authenticated queue root")?;
        let receipt = pending
            .get_receipt()
            .await
            .context("wait for queue-root receipt")?;
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
        let receipt = alloy::providers::PendingTransactionBuilder::new(
            self.api.raw_provider().root().clone(),
            tx_hash,
        )
        .get_receipt()
        .await
        .context("wait for message transaction receipt")?;
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

    async fn is_processed(&self, nonce: Hash32) -> Result<bool> {
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
                &data[64..64 + length] == message.payload,
                "MessageHandled payload mismatch"
            );
            return Ok(());
        }
        bail!("receipt did not contain exact MessageHandled event")
    }
}

fn redact_process_output(bytes: &[u8], private_key: &str) -> String {
    let mut output = String::from_utf8_lossy(bytes).into_owned();
    output = output.replace(private_key, "<redacted>");
    if let Some(raw) = private_key.strip_prefix("0x") {
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

fn receipt_value(
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
