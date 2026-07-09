# Relayer Resilience — Comprehensive Fix Proposal

**Status:** Proposal — awaiting review. No code written yet.
**Scope:** `merkle_roots/`, `message_relayer/eth_to_gear/`, `message_relayer/gear_to_eth/`, shared infra (`rpc.rs`, `common.rs`), top-level supervisor (`main.rs`).
**Target:** Layer 1 gives every worker **up to 5 retries** with backoff on transient RPC failures (replacing today's "die on first failure" or "die after 10 lifetime failures"). If all 5 are exhausted, the worker exits and the relayer dies. Layer 2 (supervisor) catches that death and **restarts the relayer in-place** without killing sibling relayers. Transient outages are survived by Layer 1; prolonged outages are survived by Layer 2. Neither layer reconnects indefinitely.

**Hard invariant (merkle_roots):** authority set sync must **never** be skipped, and the critical-threshold trigger must **never** be skipped. No fix in this proposal may let a transient RPC error abort the relayer while an authority-set change or critical threshold is pending.

---

## [S1] Root-cause summary

All three modules die from the same four root causes:

1. **Raw `?` on RPC inside main loops.** Gear/Eth RPC calls in `process()`/`run()` use `?` directly instead of the existing `rpc::retry_gear`. One transient blip → `Err` → relayer exits.
2. **Workers die permanently.** `MerkleRootSubmitter` / `StatusFetcher` die after `MAX_RETRIES=10`; the Eth `MessageSender`, `ProofComposer`, and eth_to_gear `MessageSender` attempt exactly one `reconnect()` then `return`.
3. **No restart at the supervisor.** `supervise_running_gear_eth_core_relayers` uses `select_all` — the first task to finish aborts every sibling. gear_to_eth/eth_to_gear propagate `Err` to `main`.
4. **Startup not hardened.** `latest_finalized_block`, `authority_set_id`, `initialize_contract_cursor`, and the recovery loop use raw `?` — RPC down at restart = can't start.

---

## [S2] Layer 1 — Bounded-retry helpers

### [S2.1] Eth reconnect helper with bounded retries (new, in `rpc.rs`)

Add a reusable `EthApi` reconnect helper that retries up to 5 times with exponential backoff, then returns `Err`. Callers handle the error by letting it propagate to the supervisor restart (S4).

```rust
// rpc.rs — new helper
/// Maximum reconnect attempts before giving up and surfacing the error.
pub const ETH_RECONNECT_RETRIES: u32 = 5;

/// Reconnect `eth_api`, retrying with backoff up to ETH_RECONNECT_RETRIES times.
/// Returns Ok on success; Err after all retries exhausted.
pub async fn reconnect_eth(eth_api: &mut EthApi, who: &str) -> anyhow::Result<()> {
    let policy = RetryPolicy::default();
    for attempt in 0..ETH_RECONNECT_RETRIES {
        match eth_api.reconnect().await {
            Ok(new) => {
                *eth_api = new;
                log::info!("{who}: reconnected to Ethereum (attempt {})", attempt + 1);
                return Ok(());
            }
            Err(e) => {
                let delay = policy.delay(attempt);
                log::warn!("{who}: Ethereum reconnect failed: {e}; retrying in {delay:?} ({attempt + 1}/{ETH_RECONNECT_RETRIES})");
                tokio::time::sleep(delay).await;
            }
        }
    }
    anyhow::bail!("{who}: Ethereum reconnect failed after {ETH_RECONNECT_RETRIES} attempts")
}
```

Rationale: today every Eth worker has its own "one reconnect, else die" loop. Replacing those bodies with this bounded helper is the single largest resilience win — 5 retries covers transient blips while still surfacing prolonged outages to the supervisor.

### [S2.2] Gear RPC calls inside main loops must go through `retry_gear`

Audit every `?` on a Gear RPC call inside a `process()`/`run_inner`/`try_*` body and wrap it in `rpc::retry_gear(&mut self.api_provider, "<op>", |api| ...)`. `retry_gear` already has bounded retries (exponential backoff, capped at 60s, configured via `RetryPolicy`) — only permanent errors propagate.

This is the core fix for root cause #1.

---

## [S3] Layer 1 — per-module fixes

All fixes follow the same pattern: **catch transient errors, retry up to the bounded limit, then let the error propagate** (the supervisor in S4 catches it and restarts the relayer). No infinite loops anywhere.

### [S3.1] `merkle_roots/mod.rs` — `MerkleRootRelayer`

**Fix A — make `run_inner` non-fatal on recoverable errors (root cause #1, protects the hard invariant).**

Today `run_inner` does:
```rust
match result {
    Ok(true) => continue,
    Ok(false) => return Err("channel closed"),
    Err(err) => { log::error!(...); return Err(err); }   // <-- dies immediately
}
```

Change to: on `Err`, classify with `rpc::classify_anyhow(&err)`. If **recoverable**, log + **reconnect `api_provider` once** (best-effort — the inner `retry_gear` calls have already exhausted their own 5 retries before this point) + `continue` the loop (state is saved each iteration, so this is safe). This means a single transient RPC failure in `process()` is retried at two levels: (1) the `retry_gear` wrapper around each individual RPC call (5 retries), and (2) the outer `run_inner` loop catching a residual error and continuing. Only **permanent** errors or `Ok(false)` (channel closed) exit.

```rust
Err(err) => {
    log::error!("... error processing blocks: {err}");
    if rpc::classify_anyhow(&err) == rpc::RetryDecision::Retry {
        log::warn!("... recoverable; reconnecting and resuming");
        self.api_provider.reconnect().await.ok();
        continue;
    }
    return Err(err);
}
```

**Fix B — wrap the remaining raw-`?` Gear RPC calls in `retry_gear`:**
- Startup `run()`: `latest_finalized_block` (mod.rs:336), `authority_set_id` (341).
- Recovery loop: `get_block_at`/`from_subxt_block` (383–384), `signed_by_authority_set_id` (460).
- `initialize_contract_cursor` (549–556): gear + eth calls.
- `try_proof_merkle_root`: `fetch_queue_merkle_root` (1076), `block.inclusion_proof` (1102).
- HTTP-request arm: `block_number_to_hash`/`get_block_at` (728–730).
- Authority-set-synced arm: `search_for_authority_set_block`/`get_block_at` (952–954).

**Fix C — guarantee the hard invariant explicitly.** The critical-threshold path (`try_proof_merkle_root` with `ForceGeneration::Yes`) and the authority-set-synced drain (lines 947–957) currently return `Err` straight out of `process()`. With Fix A a recoverable error no longer kills the relayer, but we additionally make these arms **catch recoverable errors locally and `continue` the select loop** (re-arming for the next tick) rather than relying solely on the outer classifier — so the block that triggered the critical threshold / authority-set change is retried on the next iteration instead of being dropped. Permanent errors still surface.

**Fix D — prover/submitter channel-close is recoverable, not fatal.** Today `prover.peof(..) == false` → `return Ok(false)` → relayer dies. After the prover/submitter is restarted by the supervisor (S3.3/S3.2), the channel re-opens. But to avoid a race where the relayer exits before the supervisor restarts the worker, change these arms from immediate fatal exit to: log + sleep a short backoff + `continue`, so the loop survives a brief worker restart window. (Bounded: if the channel stays closed indefinitely, the relayer eventually exits and the supervisor restarts the whole thing.)

### [S3.2] `merkle_roots/submitter.rs` — `MerkleRootSubmitter`

**Fix — replace `MAX_RETRIES=10` lifetime counter with per-attempt bounded retry.** In `task()` (submitter.rs:464), the loop dies after 10 *lifetime* failures. Change to: on error, reconnect Eth via `rpc::reconnect_eth` (S2.1, up to 5 attempts). On success, **reset the consecutive-failure counter to 0** so the counter tracks *consecutive* failures, not lifetime failures. After 5 consecutive reconnect failures the worker exits and the relayer dies → supervisor restarts it (S4). A prolonged but transient Eth outage that recovers within 5 attempts is now survived entirely in-process.

### [S3.3] `merkle_roots/prover.rs` — `FinalityProver` + `SharedFinalityProver`

**Fix — retry with bounded backoff instead of exiting on first error.** `FinalityProver::run` (prover.rs:263) spawns a `spawn_blocking` that logs the error and **exits**. Wrap the `process()` call in a bounded retry loop: on `Err`, log, `reconnect()` the api_provider (once, best-effort), and retry — up to 5 consecutive failures before exiting. The `generate_proof` internals already use `retry_gear` (5 retries per RPC call), so this outer loop is a backstop for anything that still escapes. If all 5 outer retries are exhausted, the prover exits → the relayer dies → supervisor restarts.

**Same fix for `SharedFinalityProver`** (prover.rs:579): wrap `process_shared_requests` in the same bounded-retry loop (5 retries). See [S9.3] for the additional error classification fix that prevents the shared prover from dying on misclassified transient errors.

### [S3.4] `merkle_roots/authority_set_sync.rs`

Already the most hardened module (catch_unwind + reconnect loop). **One fix:** the runner's reconnect (authority_set_sync.rs:354, 394) does a single `reconnect()` and `return`s on failure. Replace with `rpc::reconnect_eth`-style bounded retry (5 attempts) so a Gear outage longer than one attempt doesn't immediately kill authority-set sync — directly protecting the hard invariant. After 5 failures the sync worker exits and the supervisor restarts the relayer, which re-runs the full authority-set-sync path on startup (never skipped).

### [S3.5] `eth_to_gear` — `TxManager`, `ProofComposer`, `MessageSender`

- **`eth_to_gear/tx_manager.rs` `run()` (127):** the `process()` Err → `return Err` path is fatal. The tx_manager itself holds no connection, so errors here come from channel-close (worker death) — handled by S4 supervisor restart of the worker. Add the same classify-and-continue guard as S3.1-FixA for safety: a recoverable error continues; only channel-close exits.
- **`proof_composer.rs` `task()` (177):** replace the single-attempt Gear+Eth reconnect with `rpc::reconnect_eth` (S2.1, 5 retries) for both clients. On success, continue the loop; on failure (5 exhausted), exit → supervisor restarts.
- **`eth_to_gear/message_sender.rs` `task()` (351) + `run_inner` (165):** the "retry a request once, then abort" logic (line 173) drops messages on the second failure. Change to: on error, reconnect with bounded retry (5 attempts via `reconnect_eth`) and **re-queue** `last_request` (already saved) and loop — giving the message up to 5 retries per RPC operation instead of 2. After 5 retries the message sender exits → supervisor restarts. `update_balance_metric` should be best-effort (it currently kills on failure via `?`).

### [S3.6] `gear_to_eth` — workers and `TxManager`

- **`gear_to_eth/tx_manager.rs` `run()` (223):** same classify-and-continue guard as S3.5.
- **`common/ethereum/message_sender.rs` `task()` (104):** replace single-attempt Eth reconnect with `rpc::reconnect_eth` (S2.1, 5 retries). On failure → exit → supervisor restarts.
- **`common/ethereum/status_fetcher.rs` `task()` (113):** remove `MAX_RETRIES=10` lifetime counter; use per-attempt bounded retry (5 via `reconnect_eth`), reset counter after a successful poll. On failure → exit → supervisor restarts.
- **`common/ethereum/merkle_root_extractor.rs`:** `fetch_hash_auth_id` (90–93) returns `None` on a single Gear reconnect failure, killing the task. Replace with 5-retry bounded loop so it only returns `None` after 5 consecutive failures. The Eth reconnect loop (120–131) is already unbounded — change to 5-retry bounded loop to match.
- **`paid_token_transfers.rs::fetch_merkle_roots` (182):** the bootstrap is fire-and-forget and silently dies on error. Wrap the inner calls in retry/backoff (up to 5 attempts) so the initial merkle-root backfill actually completes instead of being silently lost.

---

## [S4] Layer 2 — Supervisor restart-on-death backstop

When a worker exhausts its 5 retries and exits, the relayer dies. Layer 2 catches that death and restarts the relayer in-place without killing sibling relayers. The restarted relayer loads persisted state from `MerkleRootStorage` / `JSONStorage` and resumes from where it left off.

### [S4.1] `main.rs` — `supervise_running_gear_eth_core_relayers`

Replace `select_all` (which kills every sibling on first exit) with a **per-task supervisor** that restarts a dead relayer in-place:

- Each relayer runs in its own `JoinHandle`. A supervisor loop awaits all handles; when one finishes, it logs the exit, **rebuilds and restarts only that relayer** (re-reading its `EffectiveRelayerConfig`, re-creating storage/proof-storage/api_provider/web-server), and continues. Siblings keep running untouched.
- **Bounded restart policy:** max 5 restarts per relayer within a rolling 10-minute window. If a relayer dies 5 times in 10 minutes it's genuinely broken — log a CRITICAL error, stop restarting it, and surface it as a permanent failure. This prevents a hot-restart loop.
- The restarted relayer re-runs `MerkleRootRelayer::run()`, which loads persisted state from `MerkleRootStorage::load()` — so in-flight merkle roots / submitted-roots are recovered. This is the existing restart path; we just stop throwing it away on the first transient death.

This requires factoring `start_gear_eth_core_relayer` so it can be called repeatedly for one config (it mostly already is). The shared `SharedAuthoritySetSync` / `SharedFinalityProver` handles survive across a single relayer's restart because they're created once at process level (main.rs:755) and passed in.

### [S4.2] `main.rs` — gear_to_eth / eth_to_gear

These call `relayer.run().await?` directly. Wrap each in a restart loop with the same bounded-restart policy: on `Err`, log, recreate the relayer (which re-loads `JSONStorage`), and re-run. The tx_managers already persist to `JSONStorage` and re-`resume()` on startup, so restart recovers in-flight transactions.

---

## [S5] What explicitly does NOT change

- The `rpc::retry_gear` classifier and `RetryPolicy` (rpc.rs) — reused as-is; its built-in bounded retries are already the right primitive.
- The `MerkleRootStorage` save/load protocol and on-disk format — unchanged; resilience relies on the existing atomic-rename save.
- The shared-worker serialization/priority logic (`SharedAuthoritySetSync`, `SharedFinalityProver`) — only their restart wrappers change.
- Channel types and the component topology — unchanged.
- No new dependencies.

---

## [S6] Verification plan

Per-file, after implementation:
1. `cargo build` clean.
2. `cargo test` — existing tests in `prover.rs`, `rpc.rs`, `main.rs` supervisor tests must pass.
3. Add unit tests:
   - `run_inner` continues on a recoverable error (inject a recoverable `anyhow::Error`, assert the loop did not return Err).
   - `reconnect_eth` retries up to 5 times then returns Err (mock reconnect failing 5 times).
   - Supervisor restarts a dead relayer and keeps siblings alive (extend the `main.rs` `dummy_running_relayer` test harness).
   - Supervisor stops restarting after 5 deaths in 10 minutes (bounded restart policy).
4. Manual/soak: kill the Gear RPC endpoint mid-run, confirm relayer logs "recoverable; resuming" and survives; bring RPC back, confirm authority-set sync completes and a critical-threshold block gets proven.
5. **Production bug regression test:** simulate the "Failed to fetch authority set id" error path by mocking `authority_set_id` to return a transient error. Verify `classify_anyhow` returns `RetryDecision::Retry` (not `Fail`). Verify the shared prover retries and recovers instead of dying permanently.

---

## [S7] Sequencing (suggested implementation order)

1. **[S9.3] Error classification fix + shared prover restart** — active production bug, highest priority. Fix `classify_anyhow` to retry unknown errors, add bounded-retry restart to `SharedFinalityProver`.
2. `rpc::reconnect_eth` helper + `classify_anyhow` exposure (S2.1) — foundation.
3. `merkle_roots/mod.rs` Fixes A–D (S3.1) — the hard-invariant module, highest priority.
4. `submitter.rs` + `prover.rs` (S3.2, S3.3) — so their channels stop closing.
5. `authority_set_sync.rs` bounded retry reconnect (S3.4).
6. eth_to_gear + gear_to_eth workers (S3.5, S3.6).
7. Supervisor restart (S4) — backstop, built last on top of the now-bounded-retry workers.
8. Chain-len-1 optimization (S9.1) + supervisor dedup (S9.2).
9. Startup catch-up cap (S9.4) — cap block listener startup replay to 14400 blocks (~24 hours).

Each step is independently shippable and independently testable.

---

## [S8] Summary of retry strategy

| Layer | Mechanism | Bound | On exhaustion |
|-------|-----------|-------|---------------|
| Per-RPC call (inner) | `rpc::retry_gear` / per-call retry loops | 5 retries, exponential backoff (1s base, 60s cap) | Error propagates to outer loop |
| Per-process iteration (middle) | `run_inner` classify-and-continue | 1 reconnect attempt (best-effort) | If still recoverable, `continue` the loop; if permanent, relayer dies |
| Per-worker (outer) | Worker exit after 5 consecutive failures | 5 consecutive failures | Worker exits → relayer dies |
| Supervisor (outermost) | Restart relayer in-place | 5 restarts per 10-minute window | CRITICAL log, stop restarting, surface as permanent failure |

This means a single transient RPC blip is retried ~5 times at the call level (inner). A prolonged outage that exhausts those 5 retries causes the worker to exit, the relayer to die, and the supervisor to restart it (outermost). The restarted relayer loads persisted state and resumes. If the relayer dies 5 more times within 10 minutes, the supervisor gives up — something is genuinely broken.

### Edge cases addressed

- **Broadcast channel `Lagged`:** `block_listener.rs` already handles `Lagged` by continuing (gap-replay + reconnect-replay). No other broadcast subscribers exist in the three target modules — this is already correct.
- **Startup RPC down:** all startup calls (`latest_finalized_block`, `authority_set_id`, `initialize_contract_cursor`) are wrapped in `retry_gear` (Fix B). If Gear/Eth is down at startup, the relayer retries 5 times then exits → supervisor restarts it. It will keep restarting until the RPC comes back, bounded by the restart policy.
- **Authority-set-sync guarantee across restarts:** the restarted relayer re-runs `MerkleRootRelayer::run()`, which calls `sync_authority_set_id` on startup. The `SharedAuthoritySetSync` worker persists its state and re-syncs. Authority-set sync is never skipped — it runs on every startup and is retried 5 times within each run.
- **Critical-threshold across restarts:** if the relayer dies while a critical-threshold proof is in-flight, the restarted relayer loads the saved state from `MerkleRootStorage` and re-evaluates the critical threshold. The `ForceGeneration::Yes` path is re-triggered because the threshold is derived from on-chain state, not in-memory flags.

---

## [S9] Additional user-requested fixes

### [S9.1] Chain-len-1 proof generation for queued/HTTP requests

**Problem:** When the HTTP handle receives a `GetMerkleRootProof` request for a block that doesn't have a proof yet, it fetches the block by number and calls `try_proof_merkle_root`. But the block may be old — the proof chain the prover must cover is long, making generation slow and unreliable. The same issue applies to queued-up batch requests: they reference specific block numbers that may be far behind the chain head.

**Fix:** Use `signed_block_after` (line 684) — which calls `grandpa_prove_finality` to find the latest GRANDPA-justified block — as the canonical way to get a block with chain length 1. The proof for the latest justified block is always the shortest possible chain.

**HTTP handle changes (mod.rs:833–885):**
1. When a `GetMerkleRootProof` request arrives and no finalized proof exists, find the latest chain head via `signed_block_after(latest_finalized_block)` instead of fetching the requested block number directly.
2. Call `try_proof_merkle_root` with the latest justified block (chain len = 1) and `ForceGeneration::Yes`, `Priority::Yes`.
3. **Recursive override:** if the proof generation fails (recoverable error), retry with the *new* latest justified block (re-call `signed_block_after`) rather than re-using the stale block. This ensures the proof always targets the shortest chain. Bound this retry by the standard 5-retry limit.
4. Store the original requesting block number in `http_requests` so the response is sent back to the caller when the proof completes (existing behavior — the proof covers the queue that includes the requested block).

**Queued batch requests (mod.rs:807–826):**
When draining `merkle_root_batch` for proof generation, before sending requests to the prover, find the latest chain head via `signed_block_after` and use it as the proof target. All batched requests share the same chain-len-1 proof target. The prover already handles batching by authority-set-id internally — this just ensures the proof chain is always length 1.

### [S9.2] Supervisor dedup — don't re-trigger the same merkle root

**Problem:** `supervise_contract_state` (line 582) runs on `supervisor_interval` and calls `try_proof_merkle_root` for the latest chain head. If the proof generation fails or is still in-flight, the next supervisor tick re-triggers the same merkle root — wasting prover resources and potentially hot-looping.

**Fix:** Track the last supervisor-triggered merkle root in the `MerkleRootRelayer` struct:
```rust
/// (block_number, merkle_root) of the last supervisor-triggered proof request.
last_supervisor_trigger: Option<(u32, H256)>,
```

In `supervise_contract_state`, before calling `try_proof_merkle_root`:
1. Check if `(block_number, merkle_root)` matches `last_supervisor_trigger`.
2. If it matches AND the existing entry in `self.roots` has status `GenerateProof` or `SubmitProof` (i.e., already in-progress), **skip** the re-trigger — log a trace message and return `Ok(())`.
3. If the status is `Failed` or the root is different, proceed with the proof request and update `last_supervisor_trigger`.
4. Clear `last_supervisor_trigger` when a proof is finalized (`finalize_merkle_root` or batch response) so the next supervisor tick can trigger a fresh request if needed.

This prevents the supervisor from hammering the prover with duplicate requests for the same root while still allowing re-triggering after a genuine failure or when a new root appears.

### [S9.4] Cap startup catch-up to 24 hours

**Problem:** When the relayer starts after being down for a long time (days/weeks), the block listener's "startup catch-up" replays ALL unprocessed blocks from storage — potentially hundreds of thousands of blocks. This takes hours and blocks the relayer from processing live blocks.

**Fix:** In `block_listener.rs` startup path (line 78), cap `from_block` to `latest_finalized - 14400` (~24 hours at ~6-second block time). If the stored `from_block` is older than this, skip directly to the capped position. Log a warning when capping occurs so operators know blocks were skipped. The "reconnect replay" path (small gaps after transient disconnects) is NOT capped.

### [S9.3] Fix shared prover death on authority-set-ID RPC failure (active production bug)

**Observed error:**
```
ERROR relayer::merkle_roots::prover > Error processing shared finality prover requests:
  shared prover authority set id failed with Permanent RPC error: Failed to fetch authority set id
```

**Root cause — two bugs:**

1. **Misclassified error.** `take_next_shared_proof_work` (prover.rs:800) calls `rpc::retry_gear` to fetch `authority_set_id(block_hash)`. The `gear_rpc_client::GearApi::authority_set_id` wraps its errors with `.context("Failed to fetch authority set id")` (gear-rpc-client/src/lib.rs:109). If the underlying RPC error is a transient transport failure (e.g., "connection closed"), `classify_anyhow` walks `err.chain()` and *should* find it — but the error may also be a non-transport Gear node error (e.g., method-not-found, internal error, state pruning) that `is_recoverable_error_text` doesn't match, so it's classified as Permanent. The fix: expand `is_recoverable_error_text` to also match common transient Gear node errors (e.g., "internal error", "state not available", "block not found") OR change the classification to default to Retry for unknown errors and only mark as Permanent for known-unrecoverable patterns (e.g., "invalid params", "method not found").

2. **No restart.** `SharedFinalityProver::new` (prover.rs:579-587) spawns a `spawn_blocking` that calls `process_shared_requests`. If it returns `Err`, the error is logged and the task exits — permanently. The channel closes, all pending requests are lost, and any relayer that tries to send a prove request gets a channel-closed error, which cascades to relayer death.

**Fix A — expand error classification (`rpc.rs`):**
Add to `is_recoverable_error_text`:
```rust
|| message.contains("internal error")
|| message.contains("state not available")
|| message.contains("block not found")
|| message.contains("not available")
```
And add a `classify_anyhow` fallback: if no recoverable pattern matches AND no known-unrecoverable pattern matches, default to `RetryDecision::Retry` (err on the side of retrying). Add a `is_permanent_error_text` check for known-bad patterns:
```rust
pub fn is_permanent_error_text(message: impl std::fmt::Display) -> bool {
    let message = message.to_string().to_ascii_lowercase();
    message.contains("invalid params")
        || message.contains("method not found")
        || message.contains("parse error")
}
```
Change `classify_anyhow` to: if `is_recoverable_error_text` → Retry, if `is_permanent_error_text` → Fail, **else → Retry** (unknown errors are retried, not permanently failed).

**Fix B — restart the shared prover (`prover.rs`):**
Wrap the `process_shared_requests` call in `SharedFinalityProver::new` (line 579) in a bounded restart loop (5 retries, matching the global pattern). On error, log, reconnect the context's api_provider (if applicable), and re-enter `process_shared_requests`. After 5 consecutive failures, log CRITICAL and exit permanently (the supervisor in S4 restarts the whole relayer).

```rust
tokio::task::spawn_blocking(move || {
    block_on(async move {
        let mut consecutive_failures = 0;
        const MAX_RETRIES: u32 = 5;
        loop {
            match process_shared_requests(&mut rx, &worker_metrics).await {
                Ok(()) => { log::info!("Shared prover exiting"); break; }
                Err(err) => {
                    consecutive_failures += 1;
                    log::error!("Shared prover error ({consecutive_failures}/{MAX_RETRIES}): {err}");
                    if consecutive_failures >= MAX_RETRIES {
                        log::error!("Shared prover: max retries exhausted, exiting permanently");
                        break;
                    }
                    tokio::time::sleep(Duration::from_secs(5 * consecutive_failures as u64)).await;
                }
            }
        }
    })
});
```

Note: the `requests` channel (`rx`) is passed by reference to `process_shared_requests`, so it survives across restarts — pending requests are NOT lost when the loop retries.
