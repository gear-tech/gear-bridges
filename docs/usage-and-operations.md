# Usage and operations

This guide is for operators and developers who already know the bridge topology and need to exercise it safely. Read [Running the bridge](running-the-bridge.md) for installation, deployment/bootstrap and journal guards, [Internals](internals.md) for process flow, and [Merkle roots](merkle-roots.md) for the outbound root lifecycle. A source release does not activate BEEFY, upgrade public actors or approve a deployment.

## Bridge directions

- Gear to Ethereum: `beefy-relay tokens-follow` authenticates signed BEEFY commitments and authority handovers, then publishes queue snapshots with MMR proofs. Separate retained delivery workers submit individual-message Merkle proofs after finalized root acceptance and maturity.
- Ethereum to Gear: `relayer eth-gear-core` updates the checkpoint-light-client from Beacon finality; delivery composes authenticated receipt/Beacon proofs for the historical proxy and VFT manager. Independent inbound proof-profile approval remains required, including the configured Deneb/Electra and network identity rules.

Commitment acceptance, queue-root publication, message execution and economic settlement are different outcomes. Neither an HTTP acknowledgement nor a processed bit proves an original transaction succeeded.

## Choose the process

| Responsibility | Executable and command |
| --- | --- |
| Commitments, authority handovers and registered root publication | `beefy-relay tokens-follow` |
| Resume an existing registered publication | `beefy-relay tokens-publish-root` |
| Read-only retained-history authentication | `beefy-relay tokens-history-audit` |
| Source status/events | `beefy-relay bridge-status`, `beefy-relay bridge-events` |
| Ethereum checkpoints to Gear | `relayer eth-gear-core` |
| Gear token messages to Ethereum | `relayer gear-eth-tokens {all-token-transfers,paid-token-transfers}` |
| Ethereum token receipts to Gear | `relayer eth-gear-tokens {all-token-transfers,paid-token-transfers}` |
| One reviewed manual delivery | `relayer gear-eth-manual`, `relayer eth-gear-manual` |
| Independent observer-role root challenge | `relayer kill-switch` |
| Gear queue maintenance | `relayer queue-cleaner` |
| Inspect finalized published roots | `relayer fetch-merkle-roots` |

The executable surface is authoritative:

~~~sh
target/release/beefy-relay --help
target/release/beefy-relay tokens-follow --help
target/release/beefy-relay tokens-publish-root --help
target/release/relayer --help
target/release/relayer gear-eth-tokens paid-token-transfers --help
target/release/relayer eth-gear-tokens paid-token-transfers --help
target/release/relayer eth-gear-core --help
~~~

## Configuration and startup

Use existing flags, environment inputs and approved deployment records, not a new configuration format. The retained `relayer` loads `.env`; its shared inputs include `GEAR_ENDPOINT`, `GEAR_MAX_RECONNECT_ATTEMPTS`, `GEAR_SURI`, `ETH_RPC`, `ETH_BEACON_RPC`, `ETH_MESSAGE_QUEUE_ADDRESS` and command-specific signer/storage arguments. Keep keys outside the repository. `beefy-relay` uses explicit source/witness/execution endpoints and protected wallet files; do not assume relayer environment variables configure it.

For outbound publication, follow the [existing bootstrap and run procedure](running-the-bridge.md#local-nodes-and-hoodi). Authenticate the original source genesis, source/destination domains, MMR boundary, current/next authority sets, client/queue/verifier bindings and deployment receipts. Keep the approved manifest and original journal directory. Hoodi is the CLI default; mainnet requires explicit `--ethereum-network mainnet` and a separately reviewed retained-state migration manifest. Owned fixtures require their existing explicit network descriptor; rehearsal evidence is not production qualification.

Give the commitment follower, root publisher and every independent Ethereum-submitting delivery/manual/observer process distinct configured fee-payer accounts. Keep deployment, checkpoint and governance authorities separate. An in-process nonce lock cannot coordinate another process or an external signing tool. Use one writer per journal and signer, including during restarts and manual recovery.

First startup should be supervised. Confirm intended networks and profiles, finalized source/archive availability, signer identity/balance, storage permissions, authenticated catch-up and listener addresses before enabling dependent workers or traffic. Reverse proof composition, checkpoint freshness and approved inbound profiles must remain qualified independently.

## Running with Docker Compose

The existing Docker image ships `relayer`, `beefy-relay` and `beefy-deploy`. It defaults to the `relayer` entrypoint; select the other executable explicitly with `--entrypoint` when needed. A deployment may wrap it in its existing supervisor or Compose definition.

1. Review the locked image/toolchain and actual deployment identities.
2. Mount protected secrets, approved manifests, private actor journals and authenticated history.
3. Give each actor separate mutable storage, signer, HTTP/metrics bindings and restart ownership.
4. Render the service definition before starting it and inspect logs/metrics after startup.

~~~sh
docker compose config
docker compose up -d
docker compose logs -f <service>
docker compose ps
docker compose stop <service>
~~~

Restart the same actor against its original state. A supervisor is not permission to replace unresolved transactions or repeatedly restart a challenged watchdog as a repair loop.

## HTTP management API

Only the paid-token delivery workers expose these operator routes; all-token workers and the BEEFY publisher do not provide a root-proof service. Bind the API to a private interface or protected network boundary. The default delivery bind is `127.0.0.1:8443`. Each listener has its own configured token and enabled channel; another listener's token/channel is not interchangeable.

| Route | Method | Body |
| --- | --- | --- |
| `/relay_messages` | POST | `{"messages":[{"block":12345,"nonce":"0x0123"}]}` |
| `/relay_transactions` | POST | `{"transactions":[{"tx_hash":"0x...32-byte-transaction-hash..."}]}` |

Send `Content-Type: application/json` and the configured `X-Token`. JSON bodies are limited to 131,072 bytes (128 KiB). Message block heights are `u32`; nonces use the serialized U256 hex string. Use the exact original finalized source block/message or Ethereum transaction hash, not a newly created transfer.

~~~sh
curl -i -sS http://127.0.0.1:8443/relay_messages \
  -H 'Content-Type: application/json' \
  -H "X-Token: $RELAYER_HTTP_TOKEN" \
  --data '{"messages":[{"block":12345,"nonce":"0x0123"}]}'

curl -i -sS http://127.0.0.1:8443/relay_transactions \
  -H 'Content-Type: application/json' \
  -H "X-Token: $RELAYER_HTTP_TOKEN" \
  --data "{\"transactions\":[{\"tx_hash\":\"$ORIGINAL_ETH_TX_HASH\"}]}"
~~~

These examples target the corresponding separate paid worker; use its actual bind and token. The server deduplicates `(block, nonce)` or the exact transaction-hash string within each request. Empty batches also succeed. Authentication does not bypass canonical queued/fee observations or durable ownership checks downstream: an early request may defer without creating a transaction UUID.

| Status | Meaning |
| --- | --- |
| 200, empty body | All entries handed to the local channel or handled as within-request duplicates; includes an empty batch. |
| 202, empty body | Some entries handled (including duplicate handling), others failed channel handoff; no per-entry results. |
| 500, empty body | No entries handled because the local receiver was unavailable. |
| 401 | Missing/incorrect/unreadable token on an enabled, valid POST request. |
| 400 | Malformed JSON or invalid request shape. |
| 413 | JSON exceeds the body limit. |
| 404 | Route not enabled on that listener, or unknown path. |

JSON extraction may reject malformed/oversized input before the handler checks authentication. HTTP acceptance is only local handoff: it does not establish fee admission, signing, finalized destination success or token/native settlement. After a timeout or partial response, reconcile original work and journals; do not create another economic request. The token does not replace TLS, network isolation or application-external rate limiting. Restrict unauthenticated Prometheus separately.

## Manual relay and recovery

Use `gear-eth-manual` or `eth-gear-manual` only for an explicitly reviewed operation. Record source transaction/message, nonce, deployment, destination, signer and reason. Inspect automated state, original signed attempts and finalized destination effects first; ensure no other process owns the work or signer.

Both manual directions require a private, stable `--storage` directory dedicated to that original operation. Reuse it after interruption. Never clear, relabel or transplant storage to obtain a new signature. Unknown replies, absent receipts, timeouts or a coincident processed bit do not authorize replacement signing. Retain original bytes/hash/nonce, prior attempts, receipt/finality evidence and failed outcomes. See the [detailed journal and settlement rules](running-the-bridge.md#shutdown-restart-and-recovery).

After delivery, authenticate the original canonical finalized receipt, exact event and same-receipt economic effects. Native queued/mailbox value is not settlement. Preserve completed sidecars and failed journals; do not delete stale retry evidence merely because another caller processed the message. Fee failure retains the original bridge nonce/transaction for fee retry, never another bridge request. Historical paid obligations remain delivery inputs, not permission to offer new priority payments.

## Observer challenge and governance response

`kill-switch` is observer-only. Configure its existing source/Ethereum/queue connections, optional `--from-eth-block`, metrics, and protected observer key through `--eth-observer-pk-path` or `ETH_OBSERVER_PK_PATH`. It needs neither an admin signer nor an HTTP repair endpoint.

It authenticates canonical Ethereum root events, including hot blocks connected to pinned finalized ancestry, and stored roots at the pinned head, then compares the exact historical finalized source root. Hot blocks are revisited until finalized. Missing/pruned source state alone cannot trigger a challenge; source bounds/finality failures and removed/inconsistent destination logs hold the cursor. The source RPC remains an operator-configured trust boundary.

Typed transient failures during challenge gas estimation reconnect and rescan without broadcasting. Challenge receipt retries retain the same sent hash. An uncertain send requires original-transaction reconciliation, not another signature. A finalized revert requires a fresh scan before another submission. Once a challenge succeeds and finalizes, the watchdog exits for separately authorized operator intervention; it never repairs a root or unpauses the queue. The challenge pause is finite (currently two days), so persistent incidents require the existing governance pause before expiry. Use the [existing administration/recovery procedure](running-the-bridge.md#beefy-expiry-and-existing-administration) under its separate authorities; do not turn supervisor restarts into automatic repair.

## Queue cleaning

`queue-cleaner` performs Gear queue maintenance after the configured overflow delay. Check the exact command's help, intended network, signer, candidate queue and timing before a reviewed run. Capture resulting events and reconcile state afterward. It is not a generic database garbage collector; do not delete retained message or transaction evidence.

## Root inspection and publication recovery

`fetch-merkle-roots` reads already-published Ethereum roots starting at `--from-eth-block`, with the configured Ethereum/queue and Gear connections. Log queries use inclusive pages of at most 100,000 blocks while retaining the original finalized destination view and inclusion/stored-root checks. It neither fetches a proof from an HTTP server nor creates one.

Use `beefy-relay tokens-history-audit` to authenticate retained follower history without a signer, with the original manifest/state and a separately owned proof directory. `bridge-status --source-rpc <source> [--at-block <height>]` and `bridge-events --source-rpc <source> --at-block <height> [--full]` inspect source observations; matching endpoint values alone are not trust approval.

A standalone `tokens-publish-root` must use the original follower registration, publisher state, wallet identity and registered output path. Stop the existing owner first; the deployment lock is exclusive. Preserve all inventoried publications and unresolved nonce ownership. See [Merkle roots](merkle-roots.md) for the executable command shape and finalized acceptance distinctions.

## Monitoring and common holds

Monitor progress as well as liveness: authenticated source/commitment height, authority handovers/client expiry, finalized root inventory/backfill, oldest pending message, queue maturity, signer balances, original transaction/receipt status, storage integrity/disk growth, RPC/archive errors, HTTP failures and observer challenges. A running process or zero exit status is not readiness.

- **Startup exits or holds:** check approved chain/genesis/profile/bootstrap, key parsing, ownership locks, directory permissions, incomplete saves and historical archive coverage. Preserve the directory and diagnose the exact error.
- **Commitments advance but a root does not:** inspect the registered source snapshot/root, covering accepted anchor, bounds/backfill, publication journal and original signed transaction. Missing history is HOLD, not permission to reset the inventory.
- **Messages remain pending:** inspect finalized root and maturity, original message membership/fee evidence, original signed receipt and same-receipt token/native settlement. Do not equate broadcast or another caller's processed bit with original success.
- **RPC or restart changes behavior:** resume the same paths and identities. Use the follower's bounded `--reconcile-once` and read-only history audit before an endpoint move. Transport recovery cannot weaken identity/finality, replace signed bytes or extend operation deadlines.

## Safe upgrade and release checklist

Stop one independently owned actor cleanly, preserve its configuration/manifests, complete journals, authenticated history, backups and logs, install the reviewed binary, then resume the same paths under supervision. Do not run overlapping old/new writers. A protocol/governance or public actor change is separately authorized, not a routine binary restart.

Historical proof stores, verifier material, old bytecode, payment history and deployment evidence remain retained for old obligations and incident analysis. Removal of active producer code does not authorize deleting those stores or rewriting retained manifests.

Before declaring readiness, require approved identities and reverse profiles, intended secret sources, distinct signers, exclusive durable ownership, consistent readiness views with no unfinished saves, authenticated catch-up and checkpoint freshness, private delivery listeners, reachable monitoring and original finalized settlement evidence. Existing historical/local observations are not a new public activation or production qualification claim.
