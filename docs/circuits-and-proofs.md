# Proofs and authentication

Vara→Ethereum uses native BEEFY commitments and Merkle Mountain Range (MMR) inclusion proofs, not a recursive ZK producer. Ethereum→Vara retains its independent Beacon light-client and receipt-proof composition. This existing page explains both authentication boundaries; its filename is historical.

The outbound implementation is [BeefyClient](../ethereum/src/beefy/BeefyClient.sol), [VaraQueueRootVerifier](../ethereum/src/VaraQueueRootVerifier.sol), [MessageQueue](../ethereum/src/MessageQueue.sol) and the existing [beefy-relay actor](../tools/beefy-relay/src/main.rs). Source changes do not migrate public actors, custody or existing deployments. See [bridge internals](internals.md) for retained-lane evidence and scheduling limits, and the [Ethereum deployment and recovery policy](../ethereum/README.md) for release gates.

## Outbound proof statement

The authentication chain is:

~~~text
finalized source queue at block B
        |
MMR leaf inserted at B+1 commits the historical queue snapshot
        |
native BEEFY commitment at C authenticates the MMR root
        |
commitment follower -> BeefyClient accepted live anchor
        |
authenticated MMR root publisher -> VaraQueueRootVerifier -> MessageQueue
        |
individual-message Merkle proof + block maturity + replay checks
        |
application/token-manager dispatch and finalized settlement evidence
~~~

These are separate proofs and acceptance stages. An authenticated queue root does not prove a UI request, correct token configuration, payment eligibility, maturity or completed transfer. A canonically mined commitment/root is not finalized acceptance; a processed nonce alone is not proof of the original transfer's economic effect.

## Commitment authentication

The [follower](../tools/beefy-relay/src/follower.rs) authenticates source/witness identity and follows ordered native BEEFY commitments. The [Ethereum relay](../tools/beefy-relay/src/ethereum.rs) uses `createFiatShamirFinalBitfield` and `submitFiatShamir`; the client also supports the interactive protocol. The Rust actor checks every available signature, while the Fiat-Shamir EVM path verifies its selected sample. Bootstrap trusts the configured current/next authority sets; a next-set update authenticates its handover through the newest MMR leaf's next-authority data.

The client supports at most 256 validators. Both EVM signature ceilings are 86, capped by `floor(N/3)+1` with native small-set quorum rules: N=59 requires 20 EVM signatures and native quorum 40; N=256 requires 86 and 171 respectively; N=2 requires both. Distinct signer positions and accounts are required, and authenticated native rosters require distinct source keys and derived EVM accounts. These limits do not qualify an actual production validator roster.

Accepted updates authenticate source freshness using the newest leaf's parent timestamp. The client expires after 24 hours without a sufficiently fresh authenticated source checkpoint. Mandatory authority handovers and the existing idle heartbeat are not optional merely because no user message is waiting. Expiry, source stalls and unresolved original transactions HOLD; none permits fabricated freshness or a verifier fallback.

The consensus sources derive from Snowbridge commit `1201293e482ef052b9c3989dcf680046704fef3d`; the separate architectural reference is `630c2081359e2417496e7f4c1a189a7eb40f7bd4`. Vara snapshot/domain/freshness and queue changes are not identical audited upstream bytecode. Synthetic interop vectors and test-only checkpoint injection are not production consensus-authentication evidence.

## Historical queue snapshot and MMR inclusion

The [protocol encoding](../tools/beefy-relay/src/protocol.rs) commits this 86-byte snapshot:

~~~text
version=2:u8 | "vara":4 | bridgeDomain:32 | timestamp:u64LE
| initialized:u8 | queueId:u64LE | root:32
~~~

The domain is Keccak256 over `vara/gear-eth-bridge-domain/v2`, source domain (32 bytes), destination chain ID (32-byte big-endian) and destination queue address (20 bytes), concatenated in order. Under the native protocol the source domain is the approved source genesis. Domain binding is one destination lane, not a destination registry or permission to reuse another deployment's bootstrap.

The outer leaf has version 0 and 113 SCALE bytes. Its inherited Solidity field `parachainHeadsRoot` contains the snapshot hash, not parachain heads. It also commits the parent identity and next-authority set. For queue block B, insertion L=B+1, first MMR insertion S and commitment C, the leaf index is L−S and leaf count C−S+1, requiring B<C and B≥S.

[Source::proof](../tools/beefy-relay/src/source.rs) reads the exact historical snapshot and checks initialization, parent identity, insertion-time authorities, geometry and signed MMR root. It must not reconstruct an old snapshot from current queue state. Archive state and offchain MMR nodes are required; unavailable or pruned evidence is HOLD, not an empty root or a mismatch. Source/witness identity, runtime code, actual first insertion and domain activation are authenticated separately from deployment approval. See the [native activation gates](runtime-upgrade-and-beefy-activation.md).

## Typed Ethereum verifier boundary

`MessageQueue.submitMerkleRoot(uint256 sourceBlock, bytes32 merkleRoot, bytes proof)` retains its external submission shape. It calls the typed [IVerifier](../ethereum/src/interfaces/IVerifier.sol) directly:

~~~solidity
function safeVerifyProof(uint256 sourceBlock, bytes32 merkleRoot, bytes calldata proof)
    external view returns (bool success);
~~~

The proof is the canonical ABI-encoded v2 envelope: proof/bridge versions, initialization, bridge domain, source timestamp, QueueId, anchor block/root, MMR leaf, sibling items and proof order. It is not raw sibling hashes or a packed field-element input. `VaraQueueRootVerifier`:

- accepts the safe entrypoint only from its configured queue on its configured destination chain;
- rejects heights above `uint32` before narrowing, avoiding source-height aliasing;
- bounds the envelope and at most 256 MMR items, rejects unused proof-order bits and requires exact canonical ABI re-encoding;
- catches malformed decoding through a self-only decoder and returns false;
- requires initialized version-2 metadata and the exact destination-bound domain;
- binds leaf parent number to the original source height and the snapshot hash to the leaf;
- requires a live client, source height at/above MMR start and strictly before the anchor, and an exact retained accepted nonzero anchor block/root;
- verifies the SCALE leaf hash and MMR inclusion against that accepted root.

A false root-proof result reverts submission with `InvalidQueueRootProof`. A signed publication tied to an older anchor cannot be rewritten or re-signed merely because the client advanced. Empty initialized queue progress uses `safeVerifyEmptyQueueProgress` and the same authentication with a zero root; it advances eligible progress only and creates neither a registered root nor maturity.

`BeefyClient` retains the last 64 genuinely accepted `(block, MMR root)` pairs from either acceptance protocol; bootstrap is not inserted. `isAcceptedAnchor(uint64,bytes32)` reports exact retained membership, and `verifyMMRLeafProof(uint64,bytes32,bytes32,bytes32[],uint256)` additionally requires a live client. Latest fields still govern commitment ordering and freshness. A later commitment does not invalidate an earlier proof until its anchor is evicted. Eviction never authorizes rewriting a signed original or releasing its nonce, and does not invalidate already finalized root publications or their history evidence.

The 64-entry window is a count, not a time guarantee. Production release requires measured inclusion and finality latency to fit inside that window under adversarial commitment rates; otherwise release remains gated pending a reviewed capacity or atomic-publication alternative.

## Publication, delivery and persistence

Use the existing `beefy-relay tokens-follow` commitment follower/root publisher or `tokens-publish-root` standalone publisher. `tokens-history-audit`, `bridge-status` and `bridge-events` provide the existing audit/status surfaces. There is no root-proof HTTP service, recursive authority-proof store or separate producer setup.

Each follower/root journal has one owner. Preserve source/client/anchor identity, original signed bytes, nonce, transaction hash, inclusion and finality archives. Receipt uncertainty does not permit another signature or nonce allocation; re-included originals remain held until the new inclusion is finalized. Authenticated external progress can satisfy unsigned work but cannot rewrite the identity or success/failure of a signed original. Use distinct EVM fee payers for every independent submitting process, including message delivery.

The immutable BEEFY publication floor is not the first published block. Historical missing nonzero roots at/above the floor still need their own publication even behind later roots or empty progress; bounded-distance catch-up uses actual initialized historical snapshots. A zero BEEFY floor rejects new publication without disabling historical redemption. Each newly registered block has its own maturity, including repeated root hashes. A queue reset prevents substituting a later cumulative root for an older queue.

`MessageQueue.processMessage` remains a separate binary Merkle inclusion check over the original message, leaf index and total leaves. It enforces the applicable 300/3600-second delay, pause/emergency controls and global nonce replay protection before dispatch. Historical registered roots, legacy root-keyed timestamps where applicable, processed nonces and custody remain retained; missing applicable maturity fails closed. Root authentication does not authorize resetting them.

Completion requires canonical finalized original evidence and the exact queue message effect plus the expected token-manager effect in that original receipt. A competitor's success, reverted replay or missing original receipt cannot complete the original operation. The retained token/manual workers and SDK use individual-message inclusion proofs, not a SNARK boundary.

## Observer challenge versus authorized recovery

The observer-only `relayer kill-switch` authenticates destination event inclusion from a pinned canonical head back to finalized ancestry, checks stored-root consistency at that head, and compares the exact finalized historical source root. Hot destination blocks are scanned before finality and revisited until finalized. Missing/pruned data alone cannot trigger a challenge; inconsistent/removed destination logs hold the scan cursor. Its configured source RPC remains an operational trust boundary, not an additional independent consensus verifier.

A qualified mismatch sends `challengeRoot()` with the observer key. The hash is retained across receipt uncertainty; a finalized revert requires a fresh scan before another submission. A finalized successful challenge exits for separately authorized operator intervention. The observer has no admin signer, root repair URL, automatic republication or unpause.

The challenge pause is finite: `CHALLENGE_ROOT_DELAY` is two days. Persistent incidents require the existing governance pause. Challenge removal, emergency-admin actions and any authenticated publication are separately authorized responses, not observer automation.

Expiry recovery is also separate: the reviewed positive-delay timelock can replace only an expired verifier with a live forward checkpoint retaining source/MMR history and queue/chain binding. It preserves floor, roots, maturity, custody and replay state, leaves claims paused, and grants no UUPS role. Recovery is trusted re-bootstrap authority: a compromised multisig can select malicious history; monotonic checks do not prove continuity with the expired set. Follow the [recovery policy](../ethereum/README.md#expiry-recovery-and-initializer-scope) and [custody-preserving migration gates](../ethereum/README.md#mainnet-custody-preserving-migration-candidate), not an ad hoc repair flow.

## Independent Ethereum→Vara proofs and GRANDPA discovery

The [checkpoint worker](../relayer/src/ethereum_checkpoints/) advances [checkpoint-light-client](../gear-programs/checkpoint-light-client/) from finalized Beacon data. Bootstrap requires an independently trusted checkpoint and genesis validators identity; the actor's network fixes the fork schedule. Sequential committee transitions, ordered ancestry, BLS verification and revision checks remain unchanged.

The inbound [proof composer](../relayer/src/message_relayer/eth_to_gear/proof_composer.rs) retains Beacon/Deneb/Electra proof composition, receipt inclusion and historical headers. [HistoricalProxy](../gear-programs/historical-proxy/app/src/service.rs) selects the appropriate event verifier; consumer/application replies and actual settlement must still be authenticated. Outer proxy success is not economic completion. Preserve original Gear dispatch/reply and per-log settlement evidence, including pending native payout and backed-refund obligations described in [internals](internals.md#ethereum-to-gear).

GRANDPA justifications, subscriptions and authority annotations remain in finalized message discovery and queue cleaning. Removing the outbound recursive proving graph does not replace source finality, remove reverse proofs or make a numeric cursor alone authenticated history.
