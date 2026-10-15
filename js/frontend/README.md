# Vara Network Bridge

## Deployment configuration

Frontend settings are compiled by Vite; changing container runtime environment variables does not update the served bundle. Supply the network and contract settings from `.env.example` at build time. Docker accepts the same settings as build arguments, including:

- `VITE_INBOUND_PROOF_PROFILES`: independently approved `MAINNET`/`TESTNET` inbound profile JSON matching the deployed contracts, programs and checkpoint trust pins. Never put secrets in frontend configuration.
- `VITE_OWNED_FIXTURE_NETWORK`: the approved local-consensus descriptor for an isolated chain-31337 fixture only. Leave unset for Hoodi and Ethereum mainnet.
- `VITE_OUTBOUND_VERIFICATION_MODES`: required `MAINNET,TESTNET` pair of `zk` or `beefy`. BEEFY hides legacy speed choices and never charges the legacy priority fee; timing depends on finality and queue maturity. This setting does not replace inbound identity approval.

Missing deployment configuration holds the entire application; missing or mismatched inbound approval disables Ethereum-to-Vara submission/claims rather than asserting settlement. A successful image build is not transfer qualification: configure the matching live stack and verify recipient balances and finalized settlement in both directions.

The existing header selector chooses Mainnet or Testnet; URL/storage selection is retained and the default remains Mainnet. Explicitly select Testnet for Hoodi/local-Vara qualification and supply only its independently approved profile; never reuse Hoodi pins as mainnet approval.

For a loopback preview with a separate GraphQL API, use the preview origin for the testnet indexer URL and set the Node-side `BRIDGE_INDEXER_PROXY=http://127.0.0.1:5450`. Vite development and preview both proxy `/graphql` (including WebSockets); absent means no proxy, and malformed/non-HTTP(S) targets fail startup. This avoids broadening backend CORS or exposing the API. Open `http://127.0.0.1:5273/?network=testnet`, verify the actual live token pairs and indexed transfer states, and connect real approved wallets. Wallet metadata uses the browser origin. Hoodi USDC is selectable; the mainnet USDC business restriction remains.

Vara queue block/message identifiers apply only to Vara-origin transfers. Ethereum-origin transfers show their native destination settlement under Timings rather than permanent loading placeholders for those source-only fields.

Transaction details re-fetch every 60 seconds until the indexer provides a settlement timestamp. History lists use the same cadence only while a loaded page contains a nonfailed transfer without `completedAt`. Unsettled Awaiting Payment and Bridging rows expose manual claim; fee recovery remains limited to Vara Awaiting Payment. Claims authenticate the configured original manager and finalized destination effects, not merely a source receipt or optimistic status.

The Docker frontend build uses a 4 GiB Node heap; the default 2 GiB heap is insufficient for the current bundle.

Vara batch submissions wait for finalized inclusion and correlate ordered command replies to the original extrinsic's queued message IDs. The generated manager decoder supplies the exact nonce and mapped Ethereum token before any fee payment; a successful original bridge reply proves the requested spend even when approval reports no change. If fee payment fails, use the reported original transaction/message/nonce to recover from history—never resubmit the bridge request. Legacy ZK priority payment requires the authenticated finalized queue-insertion block, not the request block. Ethereum wrap, approval and transfer mutations reject reverted receipts with their original hashes.

Reply waiting scans only canonical finalized blocks under one 44-minute deadline, retaining each queued message's original sender, destination and reply ID; delayed replies remain pending rather than using a hot reply. A timeout or history conflict is HOLD/reconcile, never an automatic signing retry. Ethereum receipt checks require the original submitted hash, so a successful cancellation/replacement cannot advance the bridge flow. BEEFY/manual estimates do not depend on unused legacy priority/relayer fee observations, and cached Vara estimates are bound to the source genesis, manager and payment program.

For GraphQL regeneration, set the singular Node-side `VITE_INDEXER_ADDRESS` to the approved schema endpoint and run `yarn workspace frontend codegen`. Format `src/features/history/graphql/index.ts` with the existing frontend Prettier config afterward; schema/operation artifacts remain generated, not hand-edited. The schema endpoint does not supply bridge trust approval.

Focused regression checks (repository root):

```sh
node --experimental-strip-types js/frontend/test/submission.test.ts
yarn workspace @gear-js/bridge typecheck
yarn workspace @gear-js/bridge test test/vara-to-eth.test.ts test/eth-to-vara.test.ts --mode unit
```

The SDK root-availability helper scans bounded recent-first history and closes the watcher handoff gap; it is advisory, not proof membership or claim authorization. Builds and unit checks do not qualify browser transfers: record genuine wallet-originated source transactions, exact destination balance effects and finalized settlement metadata.
