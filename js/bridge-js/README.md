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

Required fields: `nonce`, `blockNumber`, `ethereumPublicClient`, `ethereumWalletClient`, `ethereumAccount`, `gearApi`, `messageQueueAddress`, `expectedEffect`. Optional: `wait`, `statusCb`, `deadline`. A hex nonce is exactly 32 bytes, little-endian; prefer a bigint or decimal-string CLI input.

The SDK first tries the exact stored root, then finalized stored-root events in source-height order. It accepts only an original-message inclusion proof whose root equals that configured queue's stored root. A GRANDPA set-ID match is not required for a BEEFY-root claim; an unrelated earlier root cannot hide a later usable proof. Missing archival evidence reports an unavailable original claim rather than creating a new nonce or publishing a replacement root.

Signing and submission share the original deadline. Gear and local EVM accounts sign separately, so timed-out signing cannot trigger a later submission. An already admitted network request cannot be cancelled; remote EVM wallets may combine signing and broadcast in one RPC. An unresolved submission is HOLD, possibly without a known hash: reconcile that original wallet account/nonce before any further action. The SDK does not claim RPC cancellation or provide a durable wallet journal; the admitted local-key CLI retains signed bytes before broadcasting.

`success: true` requires the original successful canonical finalized transaction, exact queue calldata/value/sender, exactly one configured-queue `MessageProcessed` event with the expected nonce, destination, message hash and root-block fields, and processed/root state at that receipt pin. The unchanged event has no source or payload field: those are bound by the original message hash and submitted call. `expectedEffect` must match that packed message and its exact destination effect in the same receipt; token dispatch requires the matching manager `Bridged` event in the same queue-processing segment. Processed nonce alone, a competing transaction, a reverted replay or missing original receipt cannot establish completion. Failure returns `success: false` with the original broadcast hash when known. A first-mined receipt is insufficient.

### `waitForMerkleRootAppearedInMessageQueue`

This helper reports root availability only. Availability does not prove the original inclusion proof, transaction finality or destination effect.

## Retained Hoodi examples

These SDK-only examples do not authorize a new lane or any mainnet operation. The selected Python admission wrapper permits app writes only after the named token preflight and full warmup pass. It supplies the retained campaign credential **file paths**, holds the inherited deployment-wide lock, and executes hash-pinned compiled Node artifacts from the directory authenticated by the selected core bundle qualification beneath `$BEEFY_RUN/app-messages/`. Corrected recovery artifacts use `artifacts-recovery-1/`; the original `artifacts/` tree remains immutable historical evidence. Do not invoke mutable source examples directly for live writes.

With `OPS` bound to the bundle selected in `$BEEFY_RUN/run.json`:

```sh
OPS_PY=/Users/ukintvs/.cache/vara-beefy-ops/bin/python
"$OPS_PY" "$OPS/run-preflight.py" app --campaign-name hoodi-milestone-1 --direction eth-to-vara -- --mode=deploy --intent-id=app-deployment
"$OPS_PY" "$OPS/run-preflight.py" app --campaign-name hoodi-milestone-1 --direction eth-to-vara -- --mode=send --intent-id=inbound-send-1 --application-id=0x0000000000000000000000000000000000000000000000000000000000000001 --payload-hex=0x0001ff70696e6700
"$OPS_PY" "$OPS/run-preflight.py" app --campaign-name hoodi-milestone-1 --direction eth-to-vara -- --mode=relay --intent-id=inbound-relay-1 --source-intent=inbound-send-1
"$OPS_PY" "$OPS/run-preflight.py" app --campaign-name hoodi-milestone-1 --direction eth-to-vara -- --mode=verify --intent-id=inbound-relay-1
```

Both directions support explicit `--mode=send|relay|verify|probe`; deployment is owned by `eth-to-vara`. Send requires a nonzero 32-byte application ID and a hex payload of at most 1024 bytes. Relay/probe takes `--source-intent`; probe also takes `--case`. Resume repeats the identical original flags plus `--resume`, preserving original signed bytes, IDs and the original 44-minute deadline. Verify is signer-free. A queued builtin response, proxy `Relayed`, or already-processed rejection is not a completed delivery. Unresolved or mismatched original evidence exits nonzero with HOLD; an explicitly named expected rejection probe may exit zero.

The existing repository scripts remain `example:eth-to-vara` and `example:vara-to-eth`. Build their SWC/Node artifacts with pinned Yarn 4.9.2 using `yarn workspace @gear-js/bridge build:examples`. Artifact generation and offline checks do not establish a live Hoodi milestone PASS.

For a distinct normal-runtime application campaign, the same admitted runtime profile must match the run, selected bundle, verification, launch and supervisor records. The selected hash-pinned application artifact directory must contain the independently supplied `inbound-proof-profile.json`, listed in its manifest and covered by the qualified application-manifest digest. Consumer identity comes from the original finalized deployment and qualified Wasm/IDL, not discovery from an arbitrary RPC. Normal admission requires its own approved profile, six-asset token preflight, five-hour warmup and two authenticated handovers. Do not reuse the retained campaign UUID, name, journal or original deadline. A missing profile or unapproved deployment remains HOLD; these commands do not authorize public writes.

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

The full default `yarn test` gate remains enabled. Supply `INBOUND_PROOF_PROFILE_PATH` as an independently approved fixture profile together with the owned source/EL/Beacon/proxy fixture inputs; do not derive trusted pins from the endpoint being tested. CI materializes that file from `INBOUND_PROOF_PROFILE_JSON` with mode `0600` and fails closed when absent. Separately labeled `--mode unit` checks include the delayed-signing regression but do not qualify or bypass the live cases. Frontend completion is `onFinalized`; missing profiles/configuration remain visible HOLD before signing.

For Node 22.19.0 qualification, set `NODE_OPTIONS=--no-experimental-websocket` before the exact Yarn checks. This selects the already-installed `ws` backend for SDK-owned test WSS clients: the native Node WebSocket remained CLOSING against public Hoodi, while `ws` completed the real close handshake and exited naturally. Owned clients disable reconnect and are closed after use; caller-owned SDK clients are never closed or reconfigured. This is a transport-backend prerequisite, not warning suppression.

## Support

- [GitHub Issues](https://github.com/gear-tech/gear-bridges/issues)
- [Gear Protocol Documentation](https://wiki.gear-tech.io/)
- [Vara Network](https://vara.network/)
