# @gear-js/bridge

A TypeScript SDK for outbound Vara BEEFY/MMR queue claims and inbound Ethereum Beacon/checkpoint/receipt proofs.

## Installation

```sh
npm install @gear-js/bridge
```

## Prerequisites

Use authenticated deployment identities, not RPC URLs alone: Ethereum chain ID/genesis and Beacon genesis/fork schedule; source genesis, a canonical finalized runtime pin; configured queue/proxy, deployed endpoint/checkpoint CodeIds and exact IDL digests. Inbound preparation requires an `inboundProfile` from that qualified deployment. Only reviewed Electra/Fulu receipt blocks using the deployed Electra-compatible framing are supported by this SDK path. Unknown forks, endpoint/IDL mismatches and unavailable history stop the relay. Checkpoint waiting is bounded as described below.

The native-capable token frontend also requires approved `nativeWrapper.programId`, `nativeWrapper.codeId` and `nativeWrapper.idlSha256`; missing native pins HOLD before signing. Generic SDK/Ping profiles without native settlement may omit that binding.

Inbound `wait: false` requires the checkpoint already applied at the supplied finalized preparation pin. `wait: true` retries only `NotPresent`, polling later canonical finalized source pins while preparation is still unsigned. Each pin must preserve the original and previously observed finalized history and the exact runtime, proxy, slot-selected endpoint, checkpoint and deployed CodeIds. `OutDated`, unknown failures, changed bindings and unsupported forks HOLD. Polling and proof I/O share the same absolute original deadline; a timeout never resets it or permits later signing.

## Quick Start

Both relay functions take one parameter object. Obtain clients, original transaction/message identity and signer from the qualified deployment configuration. `consumerReply` supplies the exact deployed decoder and finalized effect verifier; `expectedEffect` describes the packed original outbound message. Neither is an outer-success shortcut. Do not put private keys in arguments or logs.

```typescript
import { relayEthToVara, relayVaraToEth } from '@gear-js/bridge';

const inbound = await relayEthToVara({
  transactionHash,
  beaconRpcUrl,
  ethereumPublicClient,
  gearApi,
  historicalProxyId,
  inboundProfile,
  consumerReply,
  clientId,
  clientServiceName,
  clientMethodName,
  signer,
  wait: true,
  deadline: originalDeadline,
});

if (inbound.error) throw new Error(JSON.stringify(inbound.error));
// Decode inbound.clientReply with the ORIGINAL consumer's deployed ABI.
// Nonempty bytes, including an inner Err, do not establish application delivery.

const outbound = await relayVaraToEth({
  nonce: originalNonce, // bigint; zero is valid
  blockNumber: originalSourceBlock,
  ethereumPublicClient,
  ethereumWalletClient,
  ethereumAccount,
  gearApi,
  messageQueueAddress,
  expectedEffect,
  deadline: originalDeadline,
  wait: true,
});
if (!outbound.success) throw outbound.error;
```

## API Reference

### `relayEthToVara`

Required fields: `transactionHash`, `beaconRpcUrl`, `ethereumPublicClient`, `gearApi`, `historicalProxyId`, `inboundProfile`, `consumerReply`, `clientId`, `clientServiceName`, `clientMethodName`, `signer`. Optional: `signerOptions`, `wait`, `statusCb`, `deadline`.

The SDK verifies the original signed request, successful runtime dispatch and queued message identity before scanning canonical finalized history for that request's original proxy reply. It rejects substituted receipts, wrong routes/senders/destinations, trailing SCALE bytes and unsuccessful runtime replies. Request inclusion or request finalization alone cannot complete the call.

`RelayResult` retains `blockHash`, `msgId`, `txHash`, `replyPayload`, `replyBlockHash`, `replyBlockNumber`, `replyMessageId` and an already-resolved `isFinalized` promise. A finalized outer error is `error?: ProxyError`, using the deployed enum names. Outer success carries the original raw `clientReply?: HexString`; there is no `ok` alias. The required `consumerReply` binds a registry, exact `Result<...>` type and IDL digest to the approved consumer. Its `verifyEffect` must reject any missing or mismatched application/economic effect at `effectBlockHash`, or at the original reply pin when no later effect pin exists. Full framing, inner result and effect validation are mandatory.

The only continuation exception is the exact token-manager `NativeSettlementPending` transition. With the approved native wrapper and exact `nativeSettlement.expectedEffect`, authenticate the original per-log cohort and original wrapper payout as `Delivered` with zero returned value, then, if the receipt is still `Reserved`, use non-economic `ReconcileReceipt` and require every row `Settled`/receipt `Processed`. If it is already `Processed`, the same original payout and all-row settlement evidence are still required, but no redundant call is signed. Native callbacks include `readReceiptStatus`, `readDeposits` and `readRedemption` at the authenticated pin. The result preserves the original pending reply and records the later authenticated effect pin; `nativeReconciliation` is present only when that call was needed. It never resubmits the proof or dispatches another mint, transfer or redemption. Arbitrary inner errors, mixed failed/ambiguous rows and unavailable original outcomes remain HOLD. The durable Rust inbound owner journals this continuation independently; the SDK still does not provide a durable wallet journal.

### `relayVaraToEth`

Required fields: `nonce`, `blockNumber`, `ethereumPublicClient`, `ethereumWalletClient`, `ethereumAccount`, `gearApi`, `messageQueueAddress`, `expectedEffect`. Optional: `messageQueueDeploymentBlock`, `wait`, `statusCb`, `deadline`. The deployment-block hint is a bigint from the original finalized queue creation receipt, not the last block in a multi-contract deployment. The SDK authenticates the code boundary and canonical range; missing or invalid hints fall back to archival discovery and never authorize a proof. A hex nonce is exactly 32 bytes, little-endian; prefer a bigint or decimal-string CLI input.

The SDK first tries the exact stored root, then canonically finalized stored-root events in bounded recent-first windows, prioritizing the authenticated deployment range when supplied and retaining an older-history fallback. It accepts only an original-message inclusion proof whose root equals that configured queue's stored root. A GRANDPA set-ID match is not required for a BEEFY-root claim; an unrelated earlier root cannot hide a later usable proof. Missing archival evidence reports an unavailable original claim rather than creating a new nonce or publishing a replacement root.

Manual claim buttons do not use indexer counts or stale availability hints as prerequisites: approved profile/account gating remains, and SDK unsigned preparation is authoritative. Each mutation sets one 44-minute absolute deadline before wallet/archive setup and opts into `wait: true`; setup promises and admitted remote requests are not cancelled by it. Only original settlement-verified success invokes completion. Inbound admission/settlement errors retain the known Gear transaction hash; outbound HOLD retains a known original Ethereum hash, never inventing one for an unresolved wallet call. Reconcile the original before retrying.

Historical token claims use exported `prepareVaraToEthRelay` and `getMessageQueueClient`: unsigned preparation authenticates the original queued message, then the frontend binds `expectedEffect.sourceActorId` to that message's source. The current manager preset and indexer metadata cannot replace the original actor; the SDK still checks the exact packed message, destination and token effect.

`InboundProofProfile.sourceBlockHash` is immutable independent approval, not a moving head. Later observations may advance only after `NotPresent` and exact binding/history authentication; original and previous pins remain checked through Beacon I/O. Outbound events and historical proofs use resolved exact source hashes, verify their header heights and recheck canonical pins; each source/Ethereum finalized observation is compared with its predecessor. Publication order is not source-height order, and equal root hashes at different heights remain distinct.

A newly backfilled root has its own block-specific maturity timestamp even when its hash matches an older mature root. `processMessage` accepts trailing `{ wait?: boolean }` after its deadline. With `wait: true`, only decoded `MerkleRootDelayNotPassed` retries unsigned simulation for the same selected height/message/proof under that deadline, reauthenticating the finalized queue root and prior anchor before each retry. Other errors HOLD immediately; signing/submission happens once, never reselected after signing.

Simulation, signing and submission share the original deadline. Gear and local EVM accounts sign separately, so timed-out simulation or signing cannot trigger a later submission. Local EVM signing computes the original transaction hash from the signed bytes before broadcast; response loss or a mismatched RPC hash retains that exact hash and never signs a replacement. An already admitted network request cannot be cancelled; remote EVM wallets may combine signing and broadcast in one RPC. An unresolved remote submission is HOLD, possibly without a known hash: reconcile that original wallet account/nonce before any further action. The SDK and generic example commands do not claim RPC cancellation or provide a durable wallet journal.

`success: true` requires the original successful canonical finalized transaction, exact queue calldata/value/sender, exactly one configured-queue `MessageProcessed` event with the expected nonce, destination, message hash and root-block fields, and processed/root state at that receipt pin. The unchanged event has no source or payload field: those are bound by the original message hash and submitted call. `expectedEffect` must match that packed message and its exact destination effect in the same receipt; token dispatch requires the matching manager `Bridged` event in the same queue-processing segment. Processed nonce alone, a competing transaction, a reverted replay or missing original receipt cannot establish completion. Failure returns `success: false` with the original broadcast hash when known. A first-mined receipt is insufficient.

### `waitForMerkleRootAppearedInMessageQueue`

This helper reports root availability only. It first checks the exact finalized stored root, then scans inclusive 2,000-block publication windows recent-first from the supplied Ethereum history floor (or genesis). It returns the lowest eligible source height in the first matching window, not a global minimum; maximum source progress is never a discovery cursor, so late historical backfills remain visible. After installing the watcher, inclusive catch-up closes the history/subscription gap. Removed and zero-root events are ignored, failures remain visible, and the watcher is released when the wait settles. Historical/live event candidates may still be unfinalized: availability does not prove an original inclusion proof, root maturity, transaction finality or destination effect. Use authenticated relay preparation and settlement verification for a claim.

## Authenticated application examples

The two small commands relay an **existing original** Ping/MessageHandler application message. They never deploy programs, create source transfers, replace nonces, manage campaigns, or generate approvals. Use only an independently approved deployment and record the expected message and absolute original deadline before execution. These are signing examples, not read-only verification or durable resume commands.

For a separate Ping deployment, persist its 32-byte salt before predicting the program ID or deploying the Ethereum counterpart. `newCtorFromCode` accepts hex or byte salts with the same identity. Do not decode the constructor’s automatic empty success reply as a `New` application result: verify the finalized initialized program, CodeId and service state.

Build with the pinned Yarn version, then pass one protected original-intent JSON file:

```sh
yarn workspace @gear-js/bridge build:examples
yarn workspace @gear-js/bridge example:eth-to-vara /absolute/path/inbound-intent.json
yarn workspace @gear-js/bridge example:vara-to-eth /absolute/path/outbound-intent.json
```

Both commands require exported `ETH_RPC_URL` (HTTP) and `VARA_WS_RPC` (WebSocket). Inbound also requires `BEACON_RPC_URL`, `GEAR_SURI_FILE` and `PING_IDL_PATH`; outbound requires `ETH_KEY_FILE`. Signer files must be regular mode-0600 files: the Gear file contains a SURI, and the Ethereum file contains a raw hex private key. No secret goes into arguments, intent JSON, logs, or the repository. Each derived signer must match the original intent's approved identity. `PING_IDL_PATH` must contain the exact checked-in `ping/ping.idl` whose SHA-256 matches the approved consumer binding.

Intent file fields (names match the entrypoint interfaces):

- **Inbound:** `transactionHash`, `applicationId`, `payload`, `deadline`, `signerPublicKey`, and `profile`. The profile is the independently approved `InboundProofProfile`, with `ethereumChainId`, `beaconGenesisTime`, and each fork's `epoch` serialized as decimal strings. Its consumer must be the deployed Ping's `Ping/SubmitReceipt` with original program/CodeId/IDL pins, not a profile learned from the RPC under test.
- **Outbound:** decimal-string `nonce` (zero is valid), `blockNumber`, `ethereumChainId`; hex `applicationId`, `payload`, `ethereumGenesisHash`, `sourceGenesisHash`, `sourceRuntimeCodeHash`, `sourceProgramId`, `sourceCodeId`, `messageQueueAddress`, `queueCodeHash`, `handlerAddress`, `handlerCodeHash`, `ethereumSender`; and `deadline`. Runtime hash is Blake2-256 of raw source `:code`; Ethereum code hashes are Keccak-256 of approved deployed code, not creation bytecode. These identities come from independent deployment qualification.

`deadline` is an absolute Unix millisecond integer retained from the original attempt, never recomputed on retry. `applicationId` is a nonzero 32-byte ID and `payload` is 0–1024 arbitrary bytes in even-length hex. Inbound accepts only the exact finalized consumer Ok and verifies the original receipt coordinates, application ID and persisted payload at the authenticated finalized effect pin. Outbound checks the exact original source, destination and packed payload before signing, then requires the original canonical finalized queue transaction, one same-receipt `MessageHandled` and matching same-pin application state. Simulation, signing, submission and readback share the original deadline. Inclusion, a queued response, proxy `Relayed`, a processed nonce or a reverted replay alone never establish delivery.

Missing inputs or mismatched evidence exit nonzero with HOLD. After any admitted request with an unresolved outcome, reconcile its original signer/account/nonce and transaction before signing again; these commands do not retain signed bytes or safely resume a pending transaction. The old campaign app, its test ownership and campaign docs were snapshotted outside the repository at `../review-campaign-archive/sdk` before removal; they are not SDK example commands or published package artifacts. `build:examples` clears only its generated demo output, and the package whitelist includes only ESM, CJS and declaration outputs.

### Indexer cutover prerequisite

Keep `js/indexer/db/migrations/1791244800000-receipt_identity.js`. A fresh database must ingest/reindex the independently approved original Ethereum receipts and finalized Gear settlement history. On an existing database, applying that schema alone leaves old transaction/log indices NULL and leaves legacy Completed flags unchanged. Before enabling authenticated completion, require an explicit authenticated backfill or full reindex and requalification of affected rows, preserving the original receipt slot, transaction index and log index. Do not infer identities from concatenated nonces, manufacture Settled/Processed evidence, or interpret legacy completion as authenticated delivery. This SDK change does not implement or claim a backfill or PostgreSQL migration/restart qualification; see the deployment cutover guidance in `docs/running-the-bridge.md`.

## How It Works

Ethereum → Vara: canonical finalized execution receipt → receipt trie/root → actual Beacon fork/header chain → applied checkpoint → historical proxy's slot-selected deployed endpoint → original finalized proxy reply → caller's exact consumer result/readback.

Vara → Ethereum: original finalized queued message/nonce → historical inclusion proof matching a stored BEEFY-authenticated root → unchanged queue processing call → canonical finalized configured-queue event and same-pin processed state → caller's destination readback.

## Dependencies

- Viem for Ethereum transactions and ABI decoding
- @gear-js/api and Sails for source-chain messages and deployed program ABIs
- @chainsafe/ssz and @lodestar/types for Beacon data
- @ethereumjs/trie for receipt proofs

## Contributing

Keep strict TypeScript types and extend the existing SDK test owners for consumer-visible boundaries. SDK consumer codec tests parse the hash-pinned canonical `api/gear/vft_manager.idl` with the SDK's declared Sails parser dependency; they do not depend on frontend build configuration. Integration tests require the owned source/EL/Beacon/proxy fixtures and the read-only Rust `js-test` generator; missing fixtures are failed checks, not skipped qualification.

`yarn workspace @gear-js/bridge build` emits ESM plus a bundled CommonJS entry and runs `test:package` against both real package exports. The CommonJS bundle includes import-only Beacon/SSZ dependencies while keeping declared peers external; renaming SWC output files alone does not make those dependencies require-compatible.

The full default `yarn test` gate remains enabled. Supply `INBOUND_PROOF_PROFILE_PATH` as an independently approved fixture profile together with the owned source/EL/Beacon/proxy fixture inputs; do not derive trusted pins from the endpoint being tested. CI materializes that file from `INBOUND_PROOF_PROFILE_JSON` with mode `0600` and fails closed when absent. Separately labeled `--mode unit` checks include the delayed-signing regression but do not qualify or bypass the live cases. Frontend completion is `onFinalized`; missing profiles/configuration remain visible HOLD before signing.

### Owned LocalFixture checkpoint networks

Existing Hoodi full-test approval remains pinned to chain 560048, its canonical execution genesis and Beacon validators root. An isolated fixture is a separate, independently approved profile, never a rewrite of a public profile or approval learned from a live proxy.

The current generated checkpoint IDL SHA-256 is `65c15d4eccb2a19a8ad240b82738a6335b0e9efd9a054340856f665addaca7e5`. LocalFixture requires this artifact. The previous checkpoint IDL digest remains qualified only for existing public-network profiles; no old approval file needs mutation. Refresh through the installed `sails-js generate <Cargo-generated-checkpoint_light_client.idl> --no-project --out <owned-scratch-dir> --yes` pipeline, then combine generated lib/types using the existing frontend inline-export convention and copy the generated Network registry/type into the SDK checkpoint adapter. Never approve an artifact by hashing live program state.

For a LocalFixture, serialize the checkpoint network as:

```json
{"localFixture":{"genesis_time":"<decimal genesis time>","genesis_validators_root":"0x<32-byte offline genesis root>"}}
```

The nested fields must equal the profile's `beaconGenesisTime` and `beaconGenesisValidatorsRoot`; `ethereumChainId` is `"31337"`. `parseInboundProofProfile` converts decimal strings (including nested `genesis_time`) to bigints and validates/deep-freezes the approved identity before I/O. Supply all seven forks in phase0/altair/bellatrix/capella/deneb/electra/fulu order: the first six have epoch `"0"`, Electra version is `0x60003137`, and Fulu epoch is `"18446744073709551615"`. Every version remains independently pinned and compared with Beacon spec. Only the standard mainnet preset (12-second slots, 32 slots/epoch, 256 epochs/committee period, 512 committee members) is supported; BLS, SSZ, receipt-trie, canonical finality, runtime/CodeId, historical endpoint and consumer-effect checks remain unchanged.

Full tests additionally require `OWNED_FIXTURE_NETWORK_PATH` pointing to an independently supplied descriptor generated from offline EL genesis and Beacon genesis/config **before startup**, not RPC observations:

```json
{"schemaVersion":1,"ethereumChainId":31337,"ethereumGenesisHash":"0x<32-byte EL genesis hash>","beaconGenesisValidatorsRoot":"0x<32-byte Beacon genesis root>","beaconGenesisTime":1234}
```

Both hashes are nonzero; the descriptor's time is a positive safe JSON integer. Unknown fields, descriptor/profile mismatches, missing approval and public transports HOLD before fixture I/O. `VARA_WS_RPC`, `ETH_RPC_URL` and `BEACON_RPC_URL` must use numeric `127.0.0.1` or `[::1]` addresses (not localhost/DNS). Setup then authenticates actual EL chain/genesis, Beacon genesis/spec and immutable checkpoint network against these independent pins. All existing fixture prerequisite variables and full live tests still apply; unit mode is not full-stack qualification.

Frontend `VITE_INBOUND_PROOF_PROFILES` retains its `MAINNET`/`TESTNET` map. Put a new LocalFixture profile only under `TESTNET` and independently supply the descriptor JSON as `VITE_OWNED_FIXTURE_NETWORK`. Its Gear node/archive and EL/Beacon transports must also be numeric loopback; existing chain ID, manager program/IDL, native wrapper and service/method approval checks are unchanged. A fixture cannot occupy the production profile slot.

With TESTNET chain ID `31337`, wallet configuration creates an explicit `Owned Ethereum Fixture` chain from the independently approved descriptor and actual numeric-loopback HTTP RPC, with no public block explorer. It never falls back to Viem's default Anvil endpoint or spoofs Hoodi. A missing inbound consumer profile can still display HOLD when the independent chain descriptor is valid; missing/malformed chain approval rejects chain configuration itself.

**Owned runtime acceptance scope:** keep genuine mainnet consensus timing (12-second slots, 32-slot epochs). SDK root selection requires Ethereum-finalized publication, whose ordinary consensus latency exceeds the queue's five-minute registration delay in this fixture. Consequently, its genuine live UI cannot reach the unsigned maturity-wait branch. The approved live acceptance exercises actual finality waiting/loading and settlement-only completion; exact immature-backfill/maturity waiting remains covered by the existing targeted SDK/queue regressions. This is an explicit reachability limit, not a successful live maturity demonstration. Do not shorten consensus timing, fabricate finalized RPC data, or deploy another queue to manufacture that branch.

For Node 22.19.0 qualification, set `NODE_OPTIONS=--no-experimental-websocket` before the exact Yarn checks. This selects the already-installed `ws` backend for SDK-owned test WSS clients: the native Node WebSocket remained CLOSING against public Hoodi, while `ws` completed the real close handshake and exited naturally. Owned clients disable reconnect and are closed after use; caller-owned SDK clients are never closed or reconfigured. This is a transport-backend prerequisite, not warning suppression.

## Support

- [GitHub Issues](https://github.com/gear-tech/gear-bridges/issues)
- [Gear Protocol Documentation](https://wiki.gear-tech.io/)
- [Vara Network](https://vara.network/)
