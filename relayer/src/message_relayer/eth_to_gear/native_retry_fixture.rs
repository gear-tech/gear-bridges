//! Owned Gear acceptance, not Ethereum/Beacon qualification. The real HistoricalProxy forwards
//! to the real manager; ONLY its CheckProofs endpoint is Alice's synthetic verifier mailbox.
//! Run ignored test `owned_native_retry_sender_journal_replay` with all of:
//! NATIVE_RETRY_SOURCE=ws://127.0.0.1:18944 NATIVE_RETRY_WITNESS=ws://127.0.0.1:18945
//! NATIVE_RETRY_GENESIS=0x596f836d5979c49f02c51ef567f58efffc6a3c479eab7fffc5cb176218ed7ee5
//! NATIVE_RETRY_OWNED=alice-only-local-fixture NATIVE_RETRY_OUTPUT=<fresh absolute directory>
//! NATIVE_RETRY_MANAGER_WASM=<current manager.opt.wasm>
//! NATIVE_RETRY_PROXY_WASM=<current historical_proxy.opt.wasm>
//! NATIVE_RETRY_V1_WASM=<exact 83eafd9e vft_vara_v1.opt.wasm>
//! NATIVE_RETRY_V1_SHA256=<independently supplied SHA256 of that Wasm, without 0x>.
//! The parent builds and executes serially. No endpoint fallback or skip-success exists.
use super::{
    super::{
        message_sender::{self, MessageSender, Request, Response, SignedSubmission},
        storage::{InboundRuntimeIdentity, JSONStorage},
    },
    *,
};
use anyhow::{ensure, Context, Result};
use eth_events_electra_client::ethereum_event_client::io::CheckProofs;
use gclient::GearApi;
use gear_common::{api_provider::ApiProvider, durable::immutable_json, native_wrapper::*};
use gear_core::ids::prelude::*;
use historical_proxy_client::{
    historical_proxy::io::Redirect,
    traits::{HistoricalProxy as _, HistoricalProxyFactory as _},
};
use primitive_types::{H160, H256};
use sails_rs::{calls::*, gclient::calls::*, ActorId, CodeId, MessageId, U256};
use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver};
use vft_manager_client::{
    traits::{VftManager as _, VftManagerFactory as _},
    vft_manager::io::SubmitReceipt,
    ReceiptDepositOutcome, ReceiptDepositState,
};
// Only routes present, with the same wire types, in the pinned v1 IDL are used.
use vft_vara_client::traits::{
    NativeEscrow as _, Vft as _, VftAdmin as _, VftNativeExchange as _, VftVaraFactory as _,
};

const RAW: u128 = 2_000_000_000_000;
const GENESIS: &str = "0x596f836d5979c49f02c51ef567f58efffc6a3c479eab7fffc5cb176218ed7ee5";

fn actor_hash(actor: ActorId) -> H256 {
    H256(actor.into_bytes())
}
fn sha256(bytes: &[u8]) -> H256 {
    H256(sp_core::hashing::sha2_256(bytes))
}

fn loopback(variable: &str) -> Result<String> {
    let value = std::env::var(variable).with_context(|| format!("missing {variable}"))?;
    let port = value
        .strip_prefix("ws://127.0.0.1:")
        .context("numeric IPv4 loopback websocket required")?;
    ensure!(
        !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) && port.parse::<u16>()? != 0,
        "invalid numeric loopback port"
    );
    Ok(value)
}

async fn upload(api: &GearApi, variable: &str, expected: Option<H256>) -> Result<CodeId> {
    let path =
        PathBuf::from(std::env::var(variable).with_context(|| format!("missing {variable}"))?);
    ensure!(
        path.is_absolute() && fs::symlink_metadata(&path)?.is_file(),
        "Wasm must be an absolute regular file"
    );
    let bytes = fs::read(path)?;
    if let Some(expected) = expected {
        ensure!(sha256(&bytes) == expected, "pinned v1 Wasm digest differs");
    }
    let code = CodeId::generate(&bytes);
    let signer: gsdk::signer::Signer = api.clone().into();
    if let Ok(saved) = signer
        .api()
        .original_code_storage_at(code, Some(api.last_block_hash().await?))
        .await
    {
        ensure!(saved == bytes, "existing Wasm bytes differ");
    } else {
        api.upload_code(&bytes).await?;
    }
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            if let Ok(saved) = signer
                .api()
                .original_code_storage_at(code, Some(api.last_block_hash().await?))
                .await
            {
                ensure!(saved == bytes, "finalized Wasm bytes differ");
                return Ok::<_, anyhow::Error>(code);
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
    .await
    .context("Wasm did not finalize")?
}

// Deliberately synthetic receipt inside an existing proof envelope: this fixture tests Gear
// custody/reconciliation, and does not assert that the modified envelope has an Ethereum proof.
fn event(
    index: u64,
    recipient: ActorId,
    tokens: &[H160],
    ethereum_manager: H160,
) -> EthToVaraEvent {
    use alloy::sol_types::SolEvent;
    use alloy_consensus::{Receipt, ReceiptWithBloom};
    use ethereum_client::abi::IERC20Manager::BridgingRequested;
    let mut event = super::tests::receipt_fixtures().next().unwrap().1;
    event.transaction_index = index;
    let logs = tokens
        .iter()
        .map(|token| {
            let deposit = BridgingRequested {
                from: [5; 20].into(),
                to: recipient.into_bytes().into(),
                token: token.0.into(),
                amount: alloy_primitives::U256::from(RAW),
            };
            alloy_primitives::Log {
                address: ethereum_manager.0.into(),
                data: deposit.encode_log_data(),
            }
        })
        .collect();
    let receipt =
        ethereum_common::utils::ReceiptEnvelope::Legacy(ReceiptWithBloom::from(Receipt {
            status: true.into(),
            cumulative_gas_used: 100_000,
            logs,
        }));
    event.receipt_rlp = alloy_rlp::encode(receipt);
    event
}

struct Driver {
    sender: MessageSender,
    io: MessageSenderIo,
    requests: UnboundedReceiver<Request>,
    _response_transport: tokio::sync::mpsc::UnboundedSender<Response>,
    composer: ProofComposerIo,
    directory: PathBuf,
    turn: usize,
}
impl Driver {
    fn new(sender: MessageSender, directory: PathBuf) -> Self {
        let (requests, received) = unbounded_channel();
        let (response_transport, responses) = unbounded_channel();
        let (proof_requests, _unused) = unbounded_channel();
        let (_proof_responses, proof_responses) = unbounded_channel();
        Self {
            sender,
            io: MessageSenderIo::new(requests, responses),
            requests: received,
            _response_transport: response_transport,
            composer: ProofComposerIo::new(proof_requests, proof_responses),
            directory,
            turn: 0,
        }
    }

    async fn request(&mut self, manager: &TransactionManager) -> Result<Request> {
        // Accelerate only the polling cadence, never the retry eligibility timestamp or evidence.
        manager.transactions_timestamp.write().await.clear();
        ensure!(
            manager.resume(&mut self.io, &mut self.composer).await?,
            "scheduler closed"
        );
        let request = self
            .requests
            .try_recv()
            .context("scheduler did not produce a request")?;
        ensure!(
            self.requests.try_recv().is_err(),
            "unexpected second signed lane request"
        );
        Ok(request)
    }

    async fn process(&mut self, api: &GearApi, request: &Request) -> Result<Response> {
        let response =
            message_sender::process_native_fixture(&mut self.sender, api, request).await?;
        self.turn += 1;
        let signed = response.signed_submission.as_ref();
        immutable_json(
            &self.directory.join(format!("sender-{:03}.json", self.turn)),
            &serde_json::json!({
                "scope": "owned Gear; synthetic CheckProofs, not Beacon qualification",
                "status": format!("{:?}", response.status), "signed": signed,
                "submission": response.submission,
            }),
        )?;
        Ok(response)
    }

    async fn step(&mut self, api: &GearApi, manager: &TransactionManager) -> Result<Response> {
        let request = self.request(manager).await?;
        let response = self.process(api, &request).await?;
        manager.finalize_transaction(response.clone()).await?;
        Ok(response)
    }
}

async fn add(manager: &TransactionManager, payload: &EthToVaraEvent) -> Result<Uuid> {
    let tx = Transaction::new(
        TxHashWithSlot {
            slot_number: crate::message_relayer::common::EthereumSlotNumber(
                payload.proof_block.block.slot,
            ),
            tx_hash: alloy_primitives::keccak256(payload.encode()),
        },
        TxStatus::ComposeProof,
    );
    let uuid = tx.uuid;
    manager.add_transaction(tx).await;
    manager.submit_message(uuid, payload.clone()).await?;
    Ok(uuid)
}

async fn restart(storage: &Arc<JSONStorage>) -> Result<TransactionManager> {
    let manager = TransactionManager::new(storage.clone());
    storage.load(&manager).await?;
    Ok(manager)
}

async fn rows(
    api: &GearApi,
    manager: ActorId,
    event: &EthToVaraEvent,
) -> Result<Vec<ReceiptDepositState>> {
    Ok(
        vft_manager_client::VftManager::new(GClientRemoting::new(api.clone()))
            .receipt_deposits(event.proof_block.block.slot, event.transaction_index)
            .at_block(api.last_block_hash().await?.0.into())
            .recv(manager)
            .await?,
    )
}

async fn nonce(sender: &MessageSender, api: &GearApi) -> Result<u64> {
    let signer: gsdk::signer::Signer = api.clone().into();
    Ok(sender
        .api_provider
        .client()
        .api
        .tx()
        .account_nonce(signer.account_id())
        .await?)
}

// Real parent Redirect and real proxy->manager forwarding. The only fabricated statement is
// the explicitly labelled CheckProofs response. Never fabricate a manager reply or settlement.
async fn answer_verifier(
    api: &GearApi,
    sender: &MessageSender,
    signed: &SignedSubmission,
    event: &EthToVaraEvent,
    directory: &Path,
    label: &str,
) -> Result<()> {
    let proxy: ActorId = sender.historical_proxy_address.0.into();
    let parent: MessageId = H256::from_str(
        signed
            .message_id
            .as_deref()
            .context("missing real Redirect message")?,
    )?
    .0
    .into();
    let verifier_child = MessageId::generate_outgoing(parent, 0);
    let consumer_child = MessageId::generate_outgoing(parent, 1);
    let expected = CheckProofs::encode_call(event.clone());
    let (message, _) = api
        .get_mailbox_messages(100)
        .await?
        .into_iter()
        .find(|(message, _)| message.id() == verifier_child)
        .context("exact original verifier child absent from Alice mailbox")?;
    ensure!(
        message.source() == proxy
            && message.destination().into_bytes() == <[u8; 32]>::from(api.account_id().clone())
            && message.payload_bytes() == expected,
        "verifier child identity/payload changed"
    );
    let reply: <CheckProofs as ActionIo>::Reply = Ok(eth_events_electra_client::CheckedProofs {
        receipt_rlp: event.receipt_rlp.clone(),
        transaction_index: event.transaction_index,
        block_number: 1,
        slot: event.proof_block.block.slot,
    });
    let mut bytes = CheckProofs::ROUTE.to_vec();
    reply.encode_to(&mut bytes);
    let (reply_id, _, reply_block) = api
        .send_reply_bytes(
            verifier_child,
            bytes.clone(),
            api.block_gas_limit()? / 100 * 95,
            0,
        )
        .await?;
    immutable_json(
        &directory.join(format!("{label}-synthetic-verifier.json")),
        &serde_json::json!({
            "scope": "synthetic Ethereum verifier response ONLY; actual proxy and manager",
            "parentRedirect": signed, "verifierChild": H256(verifier_child.into_bytes()),
            "consumerChild": H256(consumer_child.into_bytes()),
            "consumerPayload": hex::encode(SubmitReceipt::encode_call(event.proof_block.block.slot, event.transaction_index, event.receipt_rlp.clone())),
            "request": hex::encode(expected), "syntheticReply": hex::encode(bytes),
            "replyMessage": H256(reply_id.into_bytes()), "replyBlock": format!("{reply_block:#x}"),
        }),
    )?;
    Ok(())
}

async fn authenticate_forward(driver: &Driver, original: &SignedSubmission) -> Result<()> {
    let parent: MessageId = H256::from_str(
        original
            .message_id
            .as_deref()
            .context("missing parent Redirect")?,
    )?
    .0
    .into();
    let consumer_child = MessageId::generate_outgoing(parent, 1);
    let reply = original
        .finalized_reply
        .as_ref()
        .context("missing authenticated proxy reply")?;
    let mut input = reply
        .payload
        .strip_prefix(Redirect::ROUTE)
        .context("wrong proxy reply route")?;
    let decoded = <Redirect as ActionIo>::Reply::decode(&mut input)?;
    ensure!(input.is_empty(), "proxy reply has trailing data");
    let (receipt, manager_reply) = decoded.map_err(|e| anyhow::anyhow!("proxy failed: {e:?}"))?;
    let mut manager_input = manager_reply
        .strip_prefix(SubmitReceipt::ROUTE)
        .context("wrong actual manager reply route")?;
    let manager_result = <SubmitReceipt as ActionIo>::Reply::decode(&mut manager_input)?;
    ensure!(
        manager_input.is_empty(),
        "actual manager reply has trailing data"
    );
    let chain = driver.sender.api_provider.client();
    // Runtime statuses cover externally queued messages, not internal program children.
    // The authenticated successful Redirect contains the actual awaited manager reply.
    let mut dispatched_at = None;
    for number in original
        .inclusion_block_number
        .context("missing inclusion")?
        ..=reply
            .observation
            .finalized_block_number
            .context("missing reply pin")?
    {
        let hash = chain.block_number_to_hash(number).await?;
        for event in chain.metadata_events_at(hash).await?.iter() {
            if let Some(dispatched) =
                event?.as_event::<gsdk::gear::gear::events::MessagesDispatched>()?
            {
                if dispatched.statuses.iter().any(|(id, status)| {
                    id.into_bytes() == parent.into_bytes()
                        && matches!(
                            status,
                            gsdk::gear::runtime_types::gear_common::event::DispatchStatus::Success
                        )
                }) {
                    dispatched_at = Some((number, format!("{hash:#x}")));
                }
            }
        }
    }
    ensure!(
        dispatched_at.is_some(),
        "original Redirect did not complete after its manager reply"
    );
    immutable_json(
        &driver
            .directory
            .join(format!("forward-{}.json", original.extrinsic_hash)),
        &serde_json::json!({
            "parent": original.extrinsic_hash, "forwardedChild": H256(consumer_child.into_bytes()),
            "parentDispatch": dispatched_at, "receiptRlp": hex::encode(receipt),
            "actualManagerReply": hex::encode(manager_reply), "managerResult": format!("{manager_result:?}"),
            "authenticatedProxyReply": reply,
        }),
    )?;
    Ok(())
}

async fn wait_original(
    driver: &mut Driver,
    api: &GearApi,
    manager: &TransactionManager,
    uuid: Uuid,
    expect_retry: bool,
) -> Result<SignedSubmission> {
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let response = driver.step(api, manager).await?;
            if let Some(original) = response
                .signed_submission
                .as_ref()
                .filter(|s| s.finalized_reply.is_some())
            {
                authenticate_forward(driver, original).await?;
            }
            if matches!(response.status, MessageStatus::RetryableNoop { .. }) {
                ensure!(expect_retry, "unqualified sibling cohort authorized retry");
                return response
                    .signed_submission
                    .context("retry response omitted original");
            }
            if !expect_retry
                && response
                    .signed_submission
                    .as_ref()
                    .is_some_and(|signed| signed.finalized_reply.is_some())
            {
                ensure!(
                    matches!(response.status, MessageStatus::NeedsReconciliation { .. }),
                    "unknown cohort escaped HOLD"
                );
                return response.signed_submission.context("missing original");
            }
            ensure!(
                manager.transactions.read().await.contains_key(&uuid),
                "unexpected completion"
            );
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
    .await
    .context("original reply did not finalize")?
}

async fn persist_copy(storage_dir: &Path, directory: &Path, label: &str) -> Result<()> {
    let value: serde_json::Value =
        serde_json::from_slice(&fs::read(storage_dir.join("state.json"))?)?;
    immutable_json(&directory.join(format!("{label}-journal.json")), &value)
}

#[tokio::test]
#[ignore = "requires explicit owned numeric-loopback Gear/genesis/output and manager/proxy/exact-v1 Wasm environment; see module contract"]
async fn owned_native_retry_sender_journal_replay() -> Result<()> {
    tokio::time::timeout(Duration::from_secs(1800), run())
        .await
        .context("owned native fixture exceeded 30 minutes")?
}

async fn run() -> Result<()> {
    ensure!(
        std::env::var("NATIVE_RETRY_OWNED")? == "alice-only-local-fixture",
        "explicit fixture ownership required"
    );
    let source = loopback("NATIVE_RETRY_SOURCE")?;
    let witness = loopback("NATIVE_RETRY_WITNESS")?;
    ensure!(
        source != witness,
        "independent owned witness endpoint required"
    );
    ensure!(
        std::env::var("NATIVE_RETRY_GENESIS")? == GENESIS,
        "unexpected owned genesis approval"
    );
    let genesis = H256::from_str(GENESIS)?;
    let directory = PathBuf::from(std::env::var("NATIVE_RETRY_OUTPUT")?);
    ensure!(
        directory.is_absolute() && !directory.exists(),
        "fresh absolute output directory required"
    );
    let v1_sha = H256::from_str(&std::env::var("NATIVE_RETRY_V1_SHA256")?)?;
    ensure!(
        sha256(include_bytes!("../../../../api/gear/vft_vara_v1.idl"))
            == H256::from_str("afd6509bb15544b1302dcd4c5659bd77ef07c7dfb98a292f0c4a99b59c73fca9")?,
        "pinned original v1 IDL changed"
    );
    let api = GearApi::builder()
        .uri(&source)
        .suri("//Alice")
        .build()
        .await?;
    let second = GearApi::builder().uri(&witness).build().await?;
    ensure!(
        api.get_block_hash(0).await? == genesis && second.get_block_hash(0).await? == genesis,
        "owned runtime genesis differs"
    );
    let _owner =
        gear_common::signer_ownership::lock_signer(genesis, api.account_id().clone().into())?;
    fs::create_dir_all(&directory)?;
    let manager_code = upload(&api, "NATIVE_RETRY_MANAGER_WASM", None).await?;
    let proxy_code = upload(&api, "NATIVE_RETRY_PROXY_WASM", None).await?;
    let wrapper_code = upload(&api, "NATIVE_RETRY_V1_WASM", Some(v1_sha)).await?;
    let remoting = GClientRemoting::new(api.clone());
    let gas = api.block_gas_limit()? / 100 * 95;
    let alice: ActorId = <[u8; 32]>::from(api.account_id().clone()).into();
    let proxy = historical_proxy_client::HistoricalProxyFactory::new(remoting.clone())
        .new()
        .with_gas_limit(gas)
        .send_recv(proxy_code, Uuid::now_v7().as_bytes().to_vec())
        .await?;
    historical_proxy_client::HistoricalProxy::new(remoting.clone())
        .add_endpoint(0, alice)
        .with_gas_limit(gas)
        .send_recv(proxy)
        .await?;
    let mut wrappers = Vec::new();
    for _ in 0..2 {
        wrappers.push(
            vft_vara_client::VftVaraFactory::new(remoting.clone())
                .new(vft_vara_client::Mainnet::No)
                .with_gas_limit(gas)
                .send_recv(wrapper_code, Uuid::now_v7().as_bytes().to_vec())
                .await?,
        );
    }
    for &wrapper in &wrappers {
        vft_client::allocate_shards(remoting.clone(), wrapper, gas)
            .await
            .context("initializing immutable v1 fixture storage")?;
    }
    let (native, sibling) = (wrappers[0], wrappers[1]);
    let manager = vft_manager_client::VftManagerFactory::new(remoting.clone())
        .new(vft_manager_client::InitConfig {
            gear_bridge_builtin: H256::from_str(
                "0xf2816ced0b15749595392d3a18b5a2363d6fefe5b3b6153739f218151b7acdbf",
            )?
            .0
            .into(),
            historical_proxy_address: proxy,
            config: vft_manager_client::Config {
                gas_for_token_ops: 15_000_000_000,
                gas_for_reply_deposit: 15_000_000_000,
                gas_to_send_request_to_builtin: 10_000_000_000,
                gas_for_swap_token_maps: 1_500_000_000,
                reply_timeout: 100,
                fee_bridge: 0,
                fee_incoming: 0,
            },
        })
        .with_gas_limit(gas)
        .send_recv(manager_code, Uuid::now_v7().as_bytes().to_vec())
        .await?;
    let ethereum_manager = H160::repeat_byte(19);
    let native_eth = H160::repeat_byte(20);
    let sibling_eth = H160::repeat_byte(21);
    let mut admin = vft_manager_client::VftManager::new(remoting.clone());
    admin
        .update_erc_20_manager_address(ethereum_manager.0.into())
        .with_gas_limit(gas)
        .send_recv(manager)
        .await?;
    for (token, ethereum) in [(native, native_eth), (sibling, sibling_eth)] {
        admin
            .map_vara_to_eth_address(
                token,
                ethereum.0.into(),
                vft_manager_client::TokenSupply::Gear,
            )
            .with_gas_limit(gas)
            .send_recv(manager)
            .await?;
    }
    admin
        .configure_native_wrapper(Some(vft_manager_client::NativeWrapperConfig {
            actor_id: native,
            ethereum_token: native_eth.0.into(),
            code_id: wrapper_code.into_bytes().into(),
            mode: vft_manager_client::NativeWrapperMode::EscrowV1,
        }))
        .with_gas_limit(gas)
        .send_recv(manager)
        .await?;
    let mut wrapper_admin = vft_vara_client::VftAdmin::new(remoting.clone());
    wrapper_admin
        .pause()
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    vft_vara_client::NativeEscrow::new(remoting.clone())
        .configure_manager(manager)
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    wrapper_admin
        .set_burner(manager)
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    wrapper_admin
        .resume()
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    vft_vara_client::VftNativeExchange::new(remoting.clone())
        .mint()
        .with_value(RAW * 3)
        .with_gas_limit(gas)
        .send_recv(native)
        .await
        .context("funding initialized v1 fixture")?;
    ensure!(
        vft_vara_client::Vft::new(remoting.clone())
            .transfer(manager, U256::from(RAW * 3))
            .with_gas_limit(gas)
            .send_recv(native)
            .await?,
        "native funding failed"
    );
    wrapper_admin
        .pause()
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    wrapper_admin
        .pause()
        .with_gas_limit(gas)
        .send_recv(sibling)
        .await?;
    admin
        .unpause()
        .with_gas_limit(gas)
        .send_recv(manager)
        .await?;

    let manager_binding = ManagerProgramApproval {
        actor_id: actor_hash(manager),
        code_id: H256(manager_code.into_bytes()),
        wrapper_abi: ManagerWrapperAbi::CapabilityBound,
    };
    let approval = NativeWrapperApproval {
        schema_version: 1,
        gear_genesis_hash: genesis,
        source_manager: manager_binding.clone(),
        replacement_manager: manager_binding,
        wrapper: NativeWrapperBinding {
            actor_id: actor_hash(native),
            ethereum_token: native_eth,
            code_id: H256(wrapper_code.into_bytes()),
            mode: NativeWrapperMode::EscrowV1,
        },
    };
    let approval_path = directory.join("owned-only-native-approval.json");
    immutable_json(&approval_path, &approval)?;
    let digest = sha256(&fs::read(&approval_path)?);
    let approved = Arc::new(load_native_wrapper_approval(&approval_path, digest)?);
    ensure!(
        load_native_wrapper_approval(&approval_path, H256::repeat_byte(99)).is_err(),
        "wrong approval digest accepted"
    );
    let provider = ApiProvider::new(source, 0).await?;
    let sender = MessageSender::new(
        actor_hash(manager),
        SubmitReceipt::ROUTE.to_vec(),
        actor_hash(proxy),
        provider.connection(),
        "//Alice".into(),
        Some(ethereum_manager),
    )
    .with_native_wrapper_approval(Some(approved.clone()));
    let mut driver = Driver::new(sender, directory.clone());
    let initial = event(701, alice, &[native_eth], ethereum_manager);
    // Missing/wrong capability must fail before a signature is allocated.
    let before = nonce(&driver.sender, &api).await?;
    for invalid in [
        None,
        Some({
            let mut a = (*approved).clone();
            a.approval.wrapper.mode = NativeWrapperMode::HeldEscrowV2;
            Arc::new(a)
        }),
        Some({
            let mut a = (*approved).clone();
            a.approval.wrapper.code_id = H256::repeat_byte(99);
            Arc::new(a)
        }),
    ] {
        driver.sender.native_wrapper_approval = invalid;
        ensure!(
            driver
                .sender
                .prepare_submission(&api, &initial)
                .await
                .is_err(),
            "invalid capability prepared a signature"
        );
    }
    driver.sender.native_wrapper_approval = Some(approved.clone());
    ensure!(
        nonce(&driver.sender, &api).await? == before,
        "negative capability check consumed nonce"
    );
    let identity = InboundRuntimeIdentity {
        ethereum_chain_id: 31337,
        ethereum_genesis_hash: H256::repeat_byte(1),
        ethereum_start_block: 0,
        erc20_manager_address: Some(ethereum_manager),
        bridging_payment_address: None,
        gear_genesis_hash: genesis,
        vft_manager_address: actor_hash(manager),
        checkpoint_light_client_address: actor_hash(alice),
        historical_proxy_address: actor_hash(proxy),
        gear_sender: alice.into_bytes(),
    };
    let journal_dir = directory.join("main-journal");
    let storage = Arc::new(JSONStorage::new(&journal_dir));
    storage.bind_runtime_identity(identity.clone()).await?;
    let mut txm = TransactionManager::new(storage.clone());
    let uuid = add(&txm, &initial).await?;
    ensure!(
        driver
            .sender
            .receipt_retry_qualification(&api, &initial, None)
            .await?
            .is_none(),
        "unknown child qualified retry"
    );
    ensure!(
        matches!(
            driver.step(&api, &txm).await?.status,
            MessageStatus::Prepared
        ),
        "original not prepared"
    );
    let unresolved = driver.step(&api, &txm).await?;
    ensure!(
        matches!(unresolved.status, MessageStatus::NeedsReconciliation { .. }),
        "unanswered verifier did not HOLD"
    );
    let original = unresolved
        .signed_submission
        .context("missing signed original")?;
    let unknown_nonce = nonce(&driver.sender, &api).await?;
    let held = driver.step(&api, &txm).await?;
    ensure!(
        held.signed_submission
            .as_ref()
            .is_some_and(|saved| saved.same_request(&original))
            && nonce(&driver.sender, &api).await? == unknown_nonce,
        "unknown original caused replacement signing"
    );
    answer_verifier(
        &api,
        &driver.sender,
        &original,
        &initial,
        &directory,
        "first",
    )
    .await?;
    let qualified = wait_original(&mut driver, &api, &txm, uuid, true).await?;
    let pin = qualified
        .native_retry_qualification
        .as_ref()
        .context("native retry has no durable pin")?;
    ensure!(
        driver
            .sender
            .receipt_retry_qualification(&api, &initial, Some(pin))
            .await?
            .as_ref()
            == Some(pin),
        "historical retry pin changed"
    );
    let first_rows = rows(&api, manager, &initial).await?;
    ensure!(
        first_rows.len() == 1 && first_rows[0].outcome == ReceiptDepositOutcome::Retryable,
        "not a terminal native child error"
    );
    let row = first_rows[0].clone();
    ensure!(
        admin
            .receipt_retryable_native(
                initial.proof_block.block.slot,
                initial.transaction_index,
                row.log_index,
                row.operation_id,
                row.child.context("missing failed child")?
            )
            .recv(manager)
            .await?,
        "exact child predicate false"
    );
    ensure!(
        !admin
            .receipt_retryable_native(
                initial.proof_block.block.slot,
                initial.transaction_index,
                row.log_index,
                row.operation_id,
                MessageId::from([99; 32])
            )
            .recv(manager)
            .await?,
        "unknown child authorized retry"
    );
    persist_copy(&journal_dir, &directory, "qualified-before-retry").await?;
    drop(txm);
    txm = restart(&storage).await?;
    ensure!(
        txm.transactions.read().await[&uuid]
            .receipt
            .as_ref()
            .unwrap()
            .submission_attempts[0]
            .signed_submission
            == qualified,
        "restart lost original qualification"
    );

    // A real second receipt has both a native and a non-native terminal error. Its sibling
    // must not borrow native authority. It gets its own journal and never joins the retry lane.
    let sibling_event = event(702, alice, &[native_eth, sibling_eth], ethereum_manager);
    let sibling_storage = Arc::new(JSONStorage::new(directory.join("sibling-journal")));
    sibling_storage
        .bind_runtime_identity(identity.clone())
        .await?;
    let sibling_txm = TransactionManager::new(sibling_storage.clone());
    let sibling_uuid = add(&sibling_txm, &sibling_event).await?;
    driver.step(&api, &sibling_txm).await?;
    let sibling_original = driver
        .step(&api, &sibling_txm)
        .await?
        .signed_submission
        .context("sibling original absent")?;
    answer_verifier(
        &api,
        &driver.sender,
        &sibling_original,
        &sibling_event,
        &directory,
        "sibling",
    )
    .await?;
    wait_original(&mut driver, &api, &sibling_txm, sibling_uuid, false).await?;
    let siblings = rows(&api, manager, &sibling_event).await?;
    ensure!(
        siblings.len() == 2
            && siblings
                .iter()
                .all(|r| r.outcome == ReceiptDepositOutcome::Retryable)
            && siblings.iter().any(|r| !r.native),
        "non-native retryable sibling scenario not reached"
    );
    ensure!(
        driver
            .sender
            .receipt_retry_qualification(&api, &sibling_event, None)
            .await?
            .is_none(),
        "mixed retryable cohort escaped HOLD"
    );

    wrapper_admin
        .resume()
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    let retry_at = match txm.transactions.read().await[&uuid].status {
        TxStatus::RetryableSubmission { retry_at_ms, .. } => retry_at_ms,
        ref other => anyhow::bail!("expected durable retry status, got {other:?}"),
    };
    tokio::time::sleep(Duration::from_millis(
        retry_at.saturating_sub(message_sender::now_ms()) + 1,
    ))
    .await;
    txm.transactions_timestamp.write().await.clear();
    ensure!(
        txm.resume(&mut driver.io, &mut driver.composer).await?,
        "retry transition failed"
    );
    ensure!(
        driver.requests.try_recv().is_err(),
        "retry transition signed before durable unsigned state"
    );
    let prepared = driver.step(&api, &txm).await?;
    ensure!(
        matches!(prepared.status, MessageStatus::Prepared),
        "qualified retry was not prepared"
    );
    let retry = prepared.signed_submission.context("missing retry bytes")?;
    ensure!(
        retry.nonce > qualified.nonce
            && retry.receipt_key == qualified.receipt_key
            && retry.payload_hash == qualified.payload_hash,
        "retry changed operation input"
    );
    persist_copy(&journal_dir, &directory, "retry-prepared-before-broadcast").await?;
    let request = driver.request(&txm).await?;
    ensure!(
        matches!(&request, Request::SubmitPrepared { prepared, .. } if prepared.1 == retry),
        "wrong persisted retry sent"
    );
    // Genuine response loss: production sender broadcasts; do not deliver its response to TM.
    let lost = driver.process(&api, &request).await?;
    ensure!(
        matches!(lost.status, MessageStatus::NeedsReconciliation { .. }),
        "expected outstanding proxy response"
    );
    ensure!(
        tokio::time::timeout(Duration::from_millis(50), driver.io.recv())
            .await
            .is_err(),
        "lost response unexpectedly reached transaction manager"
    );
    let timeout_nonce = nonce(&driver.sender, &api).await?;
    drop(txm);
    txm = restart(&storage).await?;
    let replay_request = driver.request(&txm).await?;
    ensure!(
        matches!(&replay_request, Request::SubmitPrepared { prepared, .. } if prepared.1 == retry),
        "timeout restart did not replay identical bytes"
    );
    let replay = driver.process(&api, &replay_request).await?;
    ensure!(
        replay
            .signed_submission
            .as_ref()
            .is_some_and(|s| s.same_request(&retry))
            && nonce(&driver.sender, &api).await? == timeout_nonce,
        "timeout replay allocated a fresh nonce"
    );
    let retry_included = replay
        .signed_submission
        .clone()
        .context("missing replay original")?;
    txm.finalize_transaction(replay).await?;
    answer_verifier(
        &api,
        &driver.sender,
        &retry_included,
        &initial,
        &directory,
        "retry",
    )
    .await?;
    let queued_original = wait_original(&mut driver, &api, &txm, uuid, false).await?;
    let escrow = vft_vara_client::NativeEscrow::new(remoting.clone());
    let queued = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await?
        .context("retry did not enqueue payout")?;
    ensure!(
        queued.status == vft_vara_client::PayoutStatus::Queued
            && queued.returned_value == 0
            && queued.from == manager
            && queued.to == alice
            && queued.amount == U256::from(RAW),
        "unexpected native payout"
    );
    let retried_rows = rows(&api, manager, &initial).await?;
    ensure!(
        retried_rows[0].operation_id == row.operation_id && retried_rows[0].child != row.child,
        "same-operation retry did not retain operation identity"
    );
    ensure!(
        driver
            .sender
            .receipt_retry_qualification(&api, &initial, None)
            .await?
            .is_none(),
        "unknown queued payout authorized economic retry"
    );
    let queued_nonce = nonce(&driver.sender, &api).await?;
    let queued_hold = driver.step(&api, &txm).await?;
    ensure!(
        matches!(
            queued_hold.status,
            MessageStatus::NeedsReconciliation { .. }
        ) && queued_hold
            .signed_submission
            .as_ref()
            .is_some_and(|s| s.same_request(&queued_original) && s.native_reconciliation.is_none())
            && nonce(&driver.sender, &api).await? == queued_nonce,
        "queued payout caused a fresh request"
    );
    ensure!(
        escrow.redemption(row.operation_id).recv(native).await? == Some(queued.clone()),
        "payout changed during timeout replay"
    );
    let token = vft_vara_client::Vft::new(remoting.clone());
    ensure!(
        token.balance_of(manager).recv(native).await? == U256::from(RAW * 2)
            && token.total_supply().recv(native).await? == U256::from(RAW * 2),
        "timeout replay burned more than one native payout"
    );
    let (claimed, _) = api.claim_value(queued.child).await?;
    ensure!(claimed == RAW, "wrong original payout value claimed");
    // Poll the actual sender. It prepares and journals ReconcileReceipt before broadcasting it.
    tokio::time::timeout(Duration::from_secs(180), async {
        while !txm.completed.read().await.contains_key(&uuid) {
            driver.step(&api, &txm).await?;
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("native settlement did not complete")??;
    let paid = escrow
        .redemption(row.operation_id)
        .recv(native)
        .await?
        .context("original payout lost")?;
    ensure!(
        paid.child == queued.child
            && paid.status == vft_vara_client::PayoutStatus::Delivered
            && paid.returned_value == 0
            && escrow.outstanding_redemptions().recv(native).await? == 0,
        "native payout was replaced or not delivered"
    );
    ensure!(
        api.get_mailbox_messages(100)
            .await?
            .iter()
            .all(|(m, _)| m.id() != paid.child),
        "original payout remains claimable"
    );
    let complete = txm.completed.read().await[&uuid].clone();
    let evidence = complete.receipt.as_ref().unwrap();
    ensure!(
        evidence.submission_attempts.len() == 1
            && evidence.submission_attempts[0].signed_submission == qualified
            && evidence
                .signed_submission
                .as_ref()
                .is_some_and(|s| s.same_request(&retry) && s.native_reconciliation.is_some()),
        "settlement lost failed original or signed continuation"
    );
    persist_copy(&journal_dir, &directory, "completed").await?;
    drop(txm);
    let restart_nonce = nonce(&driver.sender, &api).await?;
    let restored = restart(&storage).await?;
    {
        let pending = restored.transactions.read().await;
        let original = pending
            .get(&uuid)
            .context("saved original disappeared on restart")?;
        ensure!(
            matches!(original.status, TxStatus::NeedsReconciliation { .. })
                && original.saved_settlement_observation == complete.saved_settlement_observation,
            "restart did not preserve the historical settlement fence"
        );
    }
    // Loaded completions deliberately remain provisional until the actual sender
    // reauthenticates the original dispatch, replies and saved settlement pin.
    driver.step(&api, &restored).await?;
    {
        let completed = restored.completed.read().await;
        let original = completed
            .get(&uuid)
            .context("completion did not reauthenticate after restart")?;
        let receipt = original
            .receipt
            .as_ref()
            .context("reauthenticated completion lost receipt")?;
        ensure!(
            receipt.signed_submission == evidence.signed_submission
                && receipt.submission_attempts.len() == 1
                && receipt.submission_attempts[0].signed_submission == qualified
                && original.saved_settlement_observation == complete.saved_settlement_observation,
            "restart replaced original transactions or settlement evidence"
        );
    }
    ensure!(
        nonce(&driver.sender, &api).await? == restart_nonce,
        "completion restart allocated a fresh nonce"
    );
    persist_copy(&journal_dir, &directory, "completed-requalified").await?;

    // Independently healed original: the saved error is reauthenticated before signing a retry.
    // Use a separate journal; the external helper below is a genuine proxy request, not a success
    // observation injected into TransactionManager.
    wrapper_admin
        .pause()
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    let healed_event = event(703, alice, &[native_eth], ethereum_manager);
    let healed_storage = Arc::new(JSONStorage::new(directory.join("healed-journal")));
    healed_storage.bind_runtime_identity(identity).await?;
    let healed_txm = TransactionManager::new(healed_storage.clone());
    let healed_uuid = add(&healed_txm, &healed_event).await?;
    driver.step(&api, &healed_txm).await?;
    let unhealed = driver
        .step(&api, &healed_txm)
        .await?
        .signed_submission
        .context("missing healed-scenario original")?;
    answer_verifier(
        &api,
        &driver.sender,
        &unhealed,
        &healed_event,
        &directory,
        "healed-first",
    )
    .await?;
    let healed_original = wait_original(&mut driver, &api, &healed_txm, healed_uuid, true).await?;
    let healed_row = rows(&api, manager, &healed_event).await?.remove(0);
    wrapper_admin
        .resume()
        .with_gas_limit(gas)
        .send_recv(native)
        .await?;
    let mut external = driver
        .sender
        .prepare_submission(&api, &healed_event)
        .await?;
    immutable_json(
        &directory.join("external-healing-original-before-broadcast.json"),
        &external,
    )?;
    driver
        .sender
        .reconcile_signed_submission(&api, &healed_event, &mut external, None)
        .await?;
    answer_verifier(
        &api,
        &driver.sender,
        &external,
        &healed_event,
        &directory,
        "external-heal",
    )
    .await?;
    let payout = tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            if let Some(payout) = escrow
                .redemption(healed_row.operation_id)
                .recv(native)
                .await?
            {
                return Ok::<_, anyhow::Error>(payout);
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
    })
    .await
    .context("external original payout missing")??;
    ensure!(
        api.claim_value(payout.child).await?.0 == RAW,
        "healed payout value differs"
    );
    // Actual manager reconciliation, then await finalized row; this is independent healing.
    admin
        .reconcile_receipt(
            healed_event.proof_block.block.slot,
            healed_event.transaction_index,
        )
        .with_gas_limit(gas)
        .send_recv(manager)
        .await?
        .map_err(|e| anyhow::anyhow!("independent reconciliation failed: {e:?}"))?;
    tokio::time::timeout(Duration::from_secs(180), async {
        loop {
            let rows = rows(&api, manager, &healed_event).await?;
            if rows.len() == 1 && rows[0].outcome == ReceiptDepositOutcome::Settled {
                break;
            }
            tokio::time::sleep(Duration::from_secs(3)).await;
        }
        Ok::<_, anyhow::Error>(())
    })
    .await
    .context("healing did not finalize")??;
    let external_status = driver
        .sender
        .reconcile_signed_submission(&api, &healed_event, &mut external, None)
        .await?
        .0;
    ensure!(
        matches!(external_status, MessageStatus::Success { .. }),
        "independent healing lacks original completion evidence"
    );
    authenticate_forward(&driver, &external).await?;
    immutable_json(
        &directory.join("external-healing-authenticated.json"),
        &external,
    )?;
    let retry_at = match healed_txm.transactions.read().await[&healed_uuid].status {
        TxStatus::RetryableSubmission { retry_at_ms, .. } => retry_at_ms,
        ref other => anyhow::bail!("healed journal lost retry original: {other:?}"),
    };
    tokio::time::sleep(Duration::from_millis(
        retry_at.saturating_sub(message_sender::now_ms()) + 1,
    ))
    .await;
    healed_txm.transactions_timestamp.write().await.clear();
    ensure!(
        healed_txm
            .resume(&mut driver.io, &mut driver.composer)
            .await?,
        "healed restart scheduler failed"
    );
    ensure!(
        driver.requests.try_recv().is_err(),
        "healed transition signed before reauthentication"
    );
    let before_healed = nonce(&driver.sender, &api).await?;
    let response = driver.step(&api, &healed_txm).await?;
    ensure!(
        matches!(response.status, MessageStatus::Success { .. })
            && response
                .signed_submission
                .as_ref()
                .is_some_and(|s| s.same_request(&healed_original))
            && nonce(&driver.sender, &api).await? == before_healed,
        "healed original created a retry"
    );
    ensure!(
        healed_txm.completed.read().await.contains_key(&healed_uuid),
        "healed original did not complete"
    );
    ensure!(
        token.balance_of(manager).recv(native).await? == U256::from(RAW)
            && token.total_supply().recv(native).await? == U256::from(RAW)
            && token.balance_of(alice).recv(native).await? == U256::zero(),
        "two original native payouts did not preserve exact wrapped custody/supply"
    );
    ensure!(
        escrow.outstanding_redemptions().recv(native).await? == 0
            && escrow.redemption(row.operation_id).recv(native).await? == Some(paid.clone()),
        "healing changed an unrelated original native payout"
    );
    let healed_completed = healed_txm.completed.read().await[&healed_uuid].clone();
    let healed_receipt = healed_completed
        .receipt
        .as_ref()
        .context("healed completion omitted receipt")?;
    let healed_replacements = usize::from(
        healed_receipt
            .signed_submission
            .as_ref()
            .is_some_and(|s| !s.same_request(&healed_original)),
    );
    ensure!(
        healed_replacements == 0 && healed_receipt.submission_attempts.len() == 1,
        "healed journal contains an extra original"
    );
    immutable_json(
        &directory.join("result.json"),
        &serde_json::json!({
            "scope": "owned Gear native state + production sender/journal; synthetic Ethereum verifier; NOT Beacon/mainnet/live-CodeId qualification",
            "gearGenesis": genesis, "manager": actor_hash(manager), "proxy": actor_hash(proxy), "wrapper": actor_hash(native),
            "managerCode": hex::encode(manager_code.into_bytes()), "proxyCode": hex::encode(proxy_code.into_bytes()),
            "v1Revision": "83eafd9e8a143fdbf9ac3ea8574ec7a47f3744a6", "v1WasmSha256": v1_sha,
            "v1Code": hex::encode(wrapper_code.into_bytes()), "approvalDigest": digest,
            "failedOriginal": qualified.extrinsic_hash, "retryOriginal": retry.extrinsic_hash,
            "operation": hex::encode(row.operation_id.0), "failedChild": format!("{:?}", row.child),
            "retryChild": format!("{:?}", retried_rows[0].child), "uniquePayoutChild": H256(paid.child.into_bytes()),
            "mainEconomicRequests": evidence.submission_attempts.len() + usize::from(evidence.signed_submission.is_some()),
            "mainNativePayouts": retried_rows.iter().filter(|r| r.operation_id == row.operation_id && r.native).count(),
            "timeoutReplayNonce": timeout_nonce,
            "healedOriginal": healed_original.extrinsic_hash, "externalHealingOriginal": external,
            "healedRelayerReplacementRequests": healed_replacements, "negativeSiblingRowsScale": hex::encode(siblings.encode()),
            "mainCompletion": complete,
        }),
    )?;
    println!("owned native retry evidence: {}", directory.display());
    Ok(())
}
