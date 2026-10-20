# MessageHandler Contract

This test application uses an independently approved bridge queue and a paired Ping program. It has no token, custody or governance role.

## Contract bindings

The constructor is `MessageHandler(address queue, bytes32 expectedVaraSource, address ethereumSender)`. All three identities are nonzero and immutable. `expectedVaraSource` is the predicted Ping program ID; `ethereumSender` is the authorized application sender.

- `sendMessage(bytes32 applicationId, bytes payload)` is nonpayable and accepts only `ethereumSender`. IDs must be nonzero, payloads may contain 0–1024 arbitrary bytes, and a sent ID cannot be sent again. `MessageRequested` indexes the ID, sender and Ping destination.
- `handleMessage(bytes32 source, bytes payload)` accepts only `queue` and the configured Ping source. The wire payload is the 32-byte application ID followed by 0–1024 raw bytes. A received ID cannot be delivered again, even through a different queue nonce. `MessageHandled` indexes the source and application ID and contains the exact suffix bytes.
- `received(applicationId)` and `payloadOf(applicationId)` expose persisted delivery. Sent and received IDs have separate replay namespaces.

Errors are `NotQueue`, `WrongSource`, `NotSender`, `InvalidPayload`, `AlreadySent(bytes32)` and `AlreadyReceived(bytes32)`. Invalid calls revert before delivery effects.

The existing `script/Deploy.s.sol:Deploy` requires explicit `MESSAGE_QUEUE`, `EXPECTED_VARA_SOURCE` and `ETHEREUM_SENDER` constructor inputs. It prints the deployed address and does not claim to create deployment-manifest files. Deployment authorization and retention of the original signed deployment intent are the caller's responsibility.

## Paired Ping ABI

Ping initialization takes the historical proxy, Ethereum emitter, Ethereum sender, Ethereum receiver and immutable `BridgeConfig`. The emitter and receiver are this MessageHandler contract. The initialization sender becomes Ping's owner. Configuration supplies the authenticated builtin, transport fee, request gas, reply deposit and timeout; initialize and receipt calls attach zero value.

`Ping/SubmitReceipt(slot: u64, transaction_index: u64, receipt_rlp: Vec<u8>)` authenticates the historical proxy, decodes the entire successful receipt, and requires exactly one correctly bound `MessageRequested`. Wrong sender/destination, malformed matching logs, ambiguous deliveries and receipt/application-ID replay return a typed error without accepting a delivery. `Received` and `PayloadOf` expose the original exact payload.

`Ping/SendMessage(application_id, payload)` requires the owner and exact configured transport fee. A fee mismatch produces a pre-effect runtime error reply and rolls back attached value. Pre-dispatch `NotOwner`, `InvalidPayload` and `AlreadyRequested` replies preserve the typed `Result` ABI and return all value attached to that call; rejecting a duplicate never refunds the original dispatch's fee. Ping retains the original pending request and builtin/reply identities; its reply hook accepts only an exact `EthMessageQueued` response from that builtin. Once dispatch is attempted, a failure or timeout does not authorize a refund: the pinned Gear send-and-reply-deposit API can fail after sending. `Outbound` returns `Pending` or `Queued`. Queue admission does not prove Ethereum application completion. An unresolved or failed dispatch remains reserved; reconcile the original instead of sending another application ID as a repair.

`js/bridge-js/example/lib.ts` contains the generated registry, an exact full-route reply decoder and read queries supporting `.atBlock(finalizedHash).call()`. Its constructor helper prepares an unsigned upload using a supplied 32-byte salt and supplied gas limit, attaches zero value, and checks `generateProgramId(generateCodeHash(code), salt)` against the upload's IDs. It returns `PreparedPingUpload`; the caller journals and finalizes the original deployment. There is no random-salt or default constructor.

## Offline qualification

Run from the bridge repository root with its pinned dependencies:

```sh
cargo build --locked -p ping --release
cargo test --locked --release -p ping --test ping -- --test-threads=1
cargo run --locked --release -p ping --bin ping-idl-gen
forge build --root js/bridge-js/js-test/contracts --force --no-cache
forge test --root js/bridge-js/js-test/contracts --match-contract MessageHandlerTest -vvv
yarn workspace @gear-js/bridge typecheck
```

The IDL generator uses native-only Sails IDL dependencies; the deployed WASM excludes them. Rust owner tests execute the compiled Ping WASM and exercise authenticated receipts, exact bytes and replay, native-value rollback, pre-dispatch typed-error refunds with caller/program/builtin balance checks, single-dispatch replay protection, strict builtin replies and late-original-reply reconciliation without refunding the original fee. Their counterpart builtin is a test actor, not evidence of a live bridge builtin. Foundry tests execute this contract's queue/source/sender checks, payload boundaries and both replay namespaces.

These checks qualify candidate code only. Live application delivery requires an independently approved deployment, the original canonical finalized transaction/reply, exact payload and same-pin effect readback. The generic SDK examples relay existing original messages only; campaign admission, journals, probes and restart qualification are not owned by this sample contract.
