# Vara Network Bridge

## Deployment configuration

Frontend settings are compiled by Vite; changing container runtime environment variables does not update the served bundle. Supply the network and contract settings from `.env.example` at build time. Docker accepts the same settings as build arguments, including:

- `VITE_INBOUND_PROOF_PROFILES`: independently approved `MAINNET`/`TESTNET` inbound profile JSON matching the deployed contracts, programs and checkpoint trust pins. Never put secrets in frontend configuration.
- `VITE_OWNED_FIXTURE_NETWORK`: the approved local-consensus descriptor for an isolated chain-31337 fixture only. Leave unset for Hoodi and Ethereum mainnet.

Missing deployment configuration holds the entire application; missing or mismatched inbound approval disables Ethereum-to-Vara submission/claims rather than asserting settlement. A successful image build is not transfer qualification: configure the matching live stack and verify recipient balances and finalized settlement in both directions.

Outbound delivery uses BEEFY-authenticated MMR roots, with timing that depends on finality and queue maturity; there is no verification-mode or speed selection. Source changes do not qualify existing public actor addresses as BEEFY deployments. Removing payment routes requires a new Gear program version, not an in-place update of a live actor. Public actor cutover and fee-custody transfer require separate authorization and deployed-code/identity qualification; existing deployment records remain historical evidence. Inbound profile approval is independent and remains required.

The existing header selector chooses Mainnet or Testnet; URL/storage selection is retained and the default remains Mainnet. Explicitly select Testnet for Hoodi/local-Vara qualification and supply only its independently approved profile; never reuse Hoodi pins as mainnet approval.

For a loopback preview with a separate GraphQL API, use the preview origin for the testnet indexer URL and set the Node-side `BRIDGE_INDEXER_PROXY=http://127.0.0.1:5450`. Vite development and preview both proxy `/graphql` (including WebSockets); absent means no proxy, and malformed/non-HTTP(S) targets fail startup. This avoids broadening backend CORS or exposing the API. Open `http://127.0.0.1:5273/?network=testnet`, verify the actual live token pairs and indexed transfer states, and connect real approved wallets. Wallet metadata uses the browser origin. Hoodi USDC is selectable; the mainnet USDC business restriction remains.

Vara queue block/message identifiers apply only to Vara-origin transfers. Ethereum-origin transfers show their native destination settlement under Timings rather than permanent loading placeholders for those source-only fields.

Transaction details re-fetch every 60 seconds until the indexer provides a settlement timestamp. History lists use the same cadence only while a loaded page contains a nonfailed transfer without `completedAt`. Unsettled Awaiting Payment and Bridging rows expose manual claim; fee recovery remains limited to Vara Awaiting Payment. Claims authenticate the configured original manager and finalized destination effects, not merely a source receipt or optimistic status.

Native payout rejection is a distinct terminal `Refunded` / GraphQL `REFUNDED` history status, displayed as **Refunded (wrapped)**, never native delivery or Completed. The existing `completedAt`, `completedAtBlock` and `completedAtTxHash` fields identify terminal resolution for either status; details label these coordinates **Refunded At** for refunds and disable further claims. The indexer accepts only the finalized original manager's `ReceiptDepositRefunded`, correlates the authenticated original Ethereum receipt/log and complete economic tuple, and retains its operation ID, wrapper, original manager child, payout child and exact full returned backing in `receipt_settlement.deposit`. Partial returns, wrong identities and conflicting terminal evidence cannot complete or refund a transfer.

Before serving the refund-aware API/UI, apply `js/indexer/db/migrations/1792022400000-native_refund.js`, restart the PostGraphile API to expose the appended status enum, then regenerate GraphQL artifacts with the command below and rebuild. Existing legacy rows are not reclassified by this migration; recover historical refunds only from authenticated finalized events and original receipt identities, never from old completion flags or balances. Rollback refuses to discard an already recorded refund.

Native inbound profiles bind the unchanged wrapper ActorId, actual CodeId, Ethereum token, explicit `escrow-v1`/`held-escrow-v2` capability and its actual IDL digest. Missing mode or a modern IDL paired with v1 code HOLDs before signing. The claim adapter exposes only the `Redemption` read shared by the two exact IDLs, qualified against both artifacts; it never probes unavailable hold/blocking routes. V1 reconciliation requires exact-full returned backing and authenticates the same original's terminal Refunded evidence afterward; partial/excess/ambiguous v1 liabilities remain blocked.

The Docker frontend build uses a 4 GiB Node heap; the default 2 GiB heap is insufficient for the current bundle.

Vara wrapping, approval and bridge requests remain one `utility.batchAll`. Submissions wait for finalized inclusion and correlate ordered command replies to the original extrinsic's queued message IDs. The generated manager decoder supplies the exact nonce and mapped Ethereum token before ordinary `PayFees(nonce)` is submitted as a separate transaction for automatic claims; manual claims never submit that fee. A successful original bridge reply proves the requested spend even when approval reports no change. If fee payment fails, use the reported original transaction/message/nonce to recover from history—never resubmit the bridge request. Ethereum wrap, approval and transfer mutations reject reverted receipts with their original hashes.

Vara finalized-inclusion waiting has a wall-clock timer within the same 44-minute submission deadline used by reply scanning. A silent subscription or stalled wallet request therefore releases the pending mutation with an unknown-outcome HOLD; a timeout does not cancel an already admitted transaction. The error includes the original signed hash when available, never an unsigned extrinsic hash. Late subscription callbacks cannot turn an expired request into success, and a late unsubscribe handle is still released.

Reply waiting scans only canonical finalized blocks under one 44-minute deadline, retaining each queued message's original sender, destination and reply ID; delayed replies remain pending rather than using a hot reply. A timeout or history conflict is HOLD/reconcile, never an automatic signing retry. Ethereum receipt checks require the original submitted hash, so a successful cancellation/replacement cannot advance the bridge flow. Manual estimates do not depend on unused relayer fees, and cached Vara estimates are bound to the source genesis, manager and payment program. Historical priority-payment state and events remain decodable for past obligations; they do not offer a new payment product.

For GraphQL regeneration, set the singular Node-side `VITE_INDEXER_ADDRESS` to the approved schema endpoint and run `yarn workspace frontend codegen`. Format `src/features/history/graphql/index.ts` with the existing frontend Prettier config afterward; schema/operation artifacts remain generated, not hand-edited. The schema endpoint does not supply bridge trust approval.

Focused regression checks (repository root):

```sh
node --experimental-strip-types js/frontend/test/submission.test.ts
node --experimental-strip-types js/frontend/test/native-profile.test.ts
yarn workspace @gear-js/bridge typecheck
yarn workspace @gear-js/bridge test test/vara-to-eth.test.ts test/eth-to-vara.test.ts --mode unit
```

The SDK root-availability helper scans bounded recent-first history and closes the watcher handoff gap; it is advisory, not proof membership or claim authorization. Builds and unit checks do not qualify browser transfers: record genuine wallet-originated source transactions, exact destination balance effects and finalized settlement metadata.
