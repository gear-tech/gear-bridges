# Vara Network Bridge

## Deployment configuration

Frontend settings are compiled by Vite; changing container runtime environment variables does not update the served bundle. Supply the network and contract settings from `.env.example` at build time. Docker accepts the same settings as build arguments, including:

- `VITE_INBOUND_PROOF_PROFILES`: independently approved `MAINNET`/`TESTNET` inbound profile JSON matching the deployed contracts, programs and checkpoint trust pins. Never put secrets in frontend configuration.
- `VITE_OWNED_FIXTURE_NETWORK`: the approved local-consensus descriptor for an isolated chain-31337 fixture only. Leave unset for Hoodi and Ethereum mainnet.

Missing deployment configuration holds the entire application; missing or mismatched inbound approval disables Ethereum-to-Vara submission/claims rather than asserting settlement. A successful image build is not transfer qualification: configure the matching live stack and verify recipient balances and finalized settlement in both directions.

Outbound delivery uses BEEFY-authenticated MMR roots, with timing that depends on finality and queue maturity; there is no verification-mode or speed selection. Source changes do not qualify existing public actor addresses as BEEFY deployments. Removing payment routes requires a new Gear program version, not an in-place update of a live actor. Public actor cutover and fee-custody transfer require separate authorization and deployed-code/identity qualification; existing deployment records remain historical evidence. Inbound profile approval is independent and remains required.

The existing header selector chooses Mainnet or Testnet; URL/storage selection is retained and the default remains Mainnet. Explicitly select Testnet for Hoodi/local-Vara qualification and supply only its independently approved profile; never reuse Hoodi pins as mainnet approval.

Manager cutover is a gated bundle/preset switch, not automatic historical-claim routing. `useRelayEthTx` selects the one manager in the configured preset and approved inbound profile; history does not supply authenticated per-transfer manager routing. Indexer manager authentication does not make the frontend route predecessor claims.

During an authorized drain, retain the approved predecessor bundle and profile and respect the source manager's pause state. Disable ordinary UI service whenever one preset cannot safely serve the transition. Resolve or classify every predecessor obligation using its original authenticated evidence and approved tooling; do not switch while the source is `Draining` or any cutover prerequisite is HOLD. Switch only after authenticated source `Retired`, complete custody/role handoff, successor `Ready`, and separately authorized activation. Rebuild Vite with the successor manager and its matching independently approved profile together; runtime environment changes cannot update the bundle. Preserve prior approved bundles/profiles as evidence.

Seamless predecessor claiming remains deferred behind this production gate; reviewer acknowledgment is still required, not presumed. This branch finalization does not execute cutover or authorize production activation.

For an owned preview with a separate GraphQL API, configure the approved HTTPS funds origin as the testnet indexer URL and set Node-side `BRIDGE_INDEXER_PROXY=http://127.0.0.1:5450`. Vite development and preview proxy `/graphql` (including WebSockets); malformed/non-HTTP(S) targets fail startup. Put the loopback preview behind the operator's HTTPS endpoint matching `VITE_FUNDS_ORIGIN`; another origin is intentionally refused. Preview reads the generated build's security headers, rather than inventing a permissive development CSP. Wallet metadata uses a self-hosted icon and the browser origin. WalletConnect's project owner must independently approve that exact origin.

Vara queue block/message identifiers apply only to Vara-origin transfers. Ethereum-origin transfers show their native destination settlement under Timings rather than permanent loading placeholders for those source-only fields.

Transaction details re-fetch every 60 seconds until the indexer provides a settlement timestamp. History lists use the same cadence only while a loaded page contains a nonfailed transfer without `completedAt`. Unsettled Awaiting Payment and Bridging rows expose manual claim; fee recovery remains limited to Vara Awaiting Payment. Claims authenticate the configured original manager and finalized destination effects, not merely a source receipt or optimistic status.

Native payout rejection is a distinct terminal `Refunded` / GraphQL `REFUNDED` history status, displayed as **Refunded (wrapped)**, never native delivery or Completed. The existing `completedAt`, `completedAtBlock` and `completedAtTxHash` fields identify terminal resolution for either status; details label these coordinates **Refunded At** for refunds and disable further claims. The indexer accepts only the finalized original manager's `ReceiptDepositRefunded`, correlates the authenticated original Ethereum receipt/log and complete economic tuple, and retains its operation ID, wrapper, original manager child, payout child and exact full returned backing in `receipt_settlement.deposit`. Partial returns, wrong identities and conflicting terminal evidence cannot complete or refund a transfer.

Before serving the refund-aware API/UI, apply `js/indexer/db/migrations/1792022400000-native_refund.js`, restart the PostGraphile API to expose the appended status enum, then regenerate GraphQL artifacts with the command below and rebuild. Existing legacy rows are not reclassified by this migration; recover historical refunds only from authenticated finalized events and original receipt identities, never from old completion flags or balances. Rollback refuses to discard an already recorded refund.

Native inbound profiles bind the unchanged wrapper ActorId, actual CodeId, Ethereum token, explicit `escrow-v1`/`held-escrow-v2` capability and its actual IDL digest. Missing mode or a modern IDL paired with v1 code HOLDs before signing. The claim adapter exposes only the `Redemption` read shared by the two exact IDLs, qualified against both artifacts; it never probes unavailable hold/blocking routes. V1 reconciliation requires exact-full returned backing and authenticates the same original's terminal Refunded evidence afterward; partial/excess/ambiguous v1 liabilities remain blocked.

The retained production route uses `EscrowV1` / `escrow-v1` only, with unchanged VFT/VFT-VARA and Ethereum token identities; `held-escrow-v2` support is for separately qualified deployments, not a wrapper migration. If replacing the manager, construct a new program rather than reusing old state in place. Require the complete admission/reservation inventory and actual v1 `OutstandingRedemptions == 0` before `BeginCutover` and again paused/drained before custody transfer; v2 blocking-zero is not a substitute. Immutable-v1 A2 partial/excess recovery, A6 hold consent and held-liability release limitations remain production blockers.

The Docker frontend build uses a 4 GiB Node heap; the default 2 GiB heap is insufficient for the current bundle.

Vara wrapping, approval and bridge requests remain one `utility.batchAll`. Submissions wait for finalized inclusion and correlate ordered command replies to the original extrinsic's queued message IDs. The generated manager decoder supplies the exact nonce and mapped Ethereum token before ordinary `PayFees(nonce)` is submitted as a separate transaction for automatic claims; manual claims never submit that fee. A successful original bridge reply proves the requested spend even when approval reports no change. If fee payment fails, use the reported original transaction/message/nonce to recover from history—never resubmit the bridge request. Ethereum wrap, approval and transfer mutations reject reverted receipts with their original hashes.

Vara finalized-inclusion waiting has a wall-clock timer within the same 44-minute submission deadline used by reply scanning. A silent subscription or stalled wallet request therefore releases the pending mutation with an unknown-outcome HOLD; a timeout does not cancel an already admitted transaction. The error includes the original signed hash when available, never an unsigned extrinsic hash. Late subscription callbacks cannot turn an expired request into success, and a late unsubscribe handle is still released.

Reply waiting scans only canonical finalized blocks under one 44-minute deadline, retaining each queued message's original sender, destination and reply ID; delayed replies remain pending rather than using a hot reply. A timeout or history conflict is HOLD/reconcile, never an automatic signing retry. Ethereum receipt checks require the original submitted hash, so a successful cancellation/replacement cannot advance the bridge flow. Manual estimates do not depend on unused relayer fees, and cached Vara estimates are bound to the source genesis, manager and payment program. Historical priority-payment state and events remain decodable for past obligations; they do not offer a new payment product.

For GraphQL regeneration, set the singular Node-side `VITE_INDEXER_ADDRESS` to the approved schema endpoint and run `yarn workspace frontend codegen`. Format `src/features/history/graphql/index.ts` with the existing frontend Prettier config afterward; schema/operation artifacts remain generated, not hand-edited. The schema endpoint does not supply bridge trust approval.

After exporting canonical Cargo IDLs, run `yarn workspace frontend generate:sails` and `yarn workspace frontend check:sails`. The pinned generator preserves advisory event handling and adapts constructor services to the installed Sails runtime. Each generated client exports `IDL_SHA256` for its exact source bytes; regeneration does not approve a deployed program or profile.

Focused regression checks (repository root):

```sh
yarn workspace frontend test
yarn workspace @gear-js/bridge typecheck
yarn workspace @gear-js/bridge test test/vara-to-eth.test.ts test/eth-to-vara.test.ts test/examples.test.ts --mode unit
```

The SDK root-availability helper scans bounded recent-first history and closes the watcher handoff gap; it is advisory, not proof membership or claim authorization. Builds and unit checks do not qualify browser transfers: record genuine wallet-originated source transactions, exact destination balance effects and finalized settlement metadata.

## Funds origin integrity and separate faucet

`yarn build:frontend` produces one closed JavaScript entry and bundled CSS. It requires independently approved `VITE_FUNDS_ORIGIN`, `VITE_FAUCET_ORIGIN` (HTTPS, distinct hostnames), and `BRIDGE_CSP_CONNECT_ORIGINS` (a JSON array of exact HTTPS/WSS origins). Include only approved RPC, Beacon, indexer, wallet, token-price and error-reporting services. Never derive this allowlist from indexer rows, API replies or user input; paths, credentials, wildcards and protocol-wide sources are rejected. Build-time values are not approvals by themselves.

The build hashes emitted JS/CSS/modulepreload entries with SHA-384, adds SRI, and emits `dist/serve.json` with the matching hash-based script CSP. It refuses extra JS chunks, external entry imports, executable public assets, workers and unbundled WASM rather than silently allowing them. Literal imports are bundled into the entry; dormant framework computed-import helpers remain in that hashed entry, but the hash-only CSP blocks additional unauthorized module loads. Entry-tag SRI is not an integrity guarantee for an arbitrary runtime import. Images/fonts are self-hosted or bundled data; remote CSS imports are rejected. Wallet CSS-in-JS/style attributes retain style-only permission; scripts never get `unsafe-inline`, general `unsafe-eval`, `self`, `strict-dynamic` or broad HTTPS authorization. Workers, objects, base URLs, embedding and form submission are disabled. GTM and session replay are not loaded. `BRIDGE_CSP_WASM_EVAL` defaults false: enable only after actual wallet/WASM browser evidence proves it necessary, retaining that evidence with the approved release.

Serve using `yarn workspace frontend serve`, or the Docker command with its equivalent script. The pinned `serve` resolves `--config serve.json` inside the served directory (`dist/` or `dist-faucet/`), not the process working directory. The generated policy applies to `/`, assets and SPA fallback routes. `vite preview` also reads those same generated headers; it is not a deployment substitute. DNS/TLS, exact approved wallet origins, reverse-proxy routes and ingress preservation of headers remain operator prerequisites. Verify real wallet connect/network switch/WASM startup, allowed RPC/WebSockets, denied foreign scripts/endpoints, tampered SRI resources and fallback routes before release. Build/unit checks do not prove browser compatibility or SRI enforcement.

`yarn build:faucet` produces a separate `dist-faucet/` entry. Docker selects it with `FRONTEND_APP=faucet`; never serve it from the funds origin. Funds token cards link to the exact configured origin with no referrer/opener and do not import the faucet or Turnstile. The faucet accepts pasted recipients and never connects a wallet, requests a signature, approves a token, or imports bridge signing contexts. No bridge wallet/local-storage/session state is transferred. Test labels do not choose payout routes.

For the existing isolated Gear/Hoodi dispenser, build with `VITE_ISOLATED_FAUCET=true`. The same service at loopback `127.0.0.1:5485` retains its keys, port ownership, authenticated journal, claim cooldowns and unresolved originals; do not initialize another dispenser or rewrite old identity records. Update only its private approved configuration with the new `origin` and the distinct `fundsOrigin`. The historical journal's original origin remains unchanged deliberately. The new entry uses only same-origin `/faucet/config`, `/faucet/status` and `/faucet/request` with redirect rejection; pending is never shown as finalized.

Install the emitted `faucet.nginx.conf` in nginx's **http context on the existing service host**, behind the approved HTTPS terminator forwarding the exact Host. Serve the complete artifact at `/srv/vara-faucet`, provision `/etc/nginx/faucet.htpasswd` with a separate faucet-only credential/realm, and independently validate/reload nginx under deployment authorization. The generated listener is loopback `5486`. `/faucet/*` strips credentials/cookies, overwrites the authenticated-user header only after Basic auth, rejects foreign/missing POST Origin and disables upstream retries. The service also requires that authentication header and its configured exact POST Origin. Never expose loopback `5485`, share funds credentials, forward funds sessions, set parent-domain cookies, or keep the old funds-origin `/faucet/*` proxy enabled. This source change does not alter the running host.

For the existing public captcha-backed payout service, leave isolated mode false. Provide `VITE_FAUCET_PROFILE` as `{ "genesis": "0x…", "tokens": [{ "name": "Test token", "address": "0x…" }] }` using independently approved test-chain/token identities; no ticker grants a capability. Provide the approved `VITE_TURNSTILE_SITEKEY`, `VITE_TURNSTILE_HOSTNAME` matching the faucet hostname, and Node-only `BRIDGE_FAUCET_PUBLIC_UPSTREAM` (existing HTTPS payout origin). The same-origin proxy strips `/faucet/`, so `/faucet/bridge/request` reaches the existing `/bridge/request` route. The upstream operator must approve the new Origin/site key and validate Turnstile hostname/site responses before release. Only this separate public-faucet CSP permits `https://challenges.cloudflare.com` scripts/frames. Isolated faucet CSP has no such exception. Test VARA/ERC20 requests and the external Hoodi ETH faucet link remain available; API acceptance is not finalized payout proof.

Reconstructed HTML7fe1b032 isolation checks (not the unavailable original PoC bundle):

```sh
yarn workspace frontend test:policy
node --experimental-strip-types js/frontend/src/features/faucet/isolated-api.check.mjs
node --test tools/test-faucet.test.cjs
yarn build:faucet
```

CI's `.invalid` origins are offline build fixtures, never production approval. Record the actual candidate SHA, artifact digests and browser evidence independently.

## Approved token identity

Active indexer pairs are checked against finalized approved manager mappings, code and runtime identity before they enable bridging. Native capabilities require the exact profile's `nativeWrapper`/`nativeEth` pair and code identity, never names, symbols or indexer-selected targets. Missing independent native-ETH code/pair approval disables native ETH. The frontend refreshes qualification every 30 seconds and on focus/reconnect, holds on stale/failed refresh, and rechecks the selected pair immediately before wallet operations. Permit selection has been removed in favor of exact-amount approval of the selected, validated token.
