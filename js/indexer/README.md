# Indexer evidence boundaries

Run schema migrations before either processor or the API. Migration
`1792972800000-indexer_boundaries.js` preserves existing transfer destinations and
program roles, permits an absent destination for unresolved inbound liabilities,
and maps the existing `gear_programs` table into the processor's transaction.
Rollback refuses while any null-destination liability remains. Never delete rows
or invent token identities to make that guard pass.

## Dispatch and transactions

Queued `RequestBridging` inputs are untrusted. Malformed SCALE prefixes, bodies,
noncanonical encodings and trailing bytes are logged with block, event, message
and transaction coordinates, then skipped before initiation projection. A valid
following request is still processed. Only decoding is inside this skip boundary;
projection and database errors abort processing.

Events/replies from authenticated manager, historical-proxy and payment roles
are a different boundary. Relevant decode errors abort the complete processor
callback with contextual HOLD. Sails payloads and consumed Ethereum event
payloads must round-trip exactly; extra bytes/topics are not ignored. Role
rotations use the callback's Store, not an independent PostgreSQL connection.
Subsequent events in that callback see the staged role; every retry reloads the
committed role table. Pair synchronization also repeats after a failed callback.
A call to `BatchState.save()` does **not** commit a Subsquid transaction: effects,
role rotations, SQL finality checkpoints and the processor cursor commit together.

Active pairs require consistent supply and one-to-one token identities. Inheritor
rotation registers the actual successor program, not the exited program. Retained
inactive pairs remain historical evidence. Newly observed Ethereum deposits use
`eth:<block>:<transaction-index>:<receipt-local-log-index>` IDs and a hash of the
colon-delimited tuple for their nonce. Coordinate-resolved historical IDs, nonces,
authorizations and dispositions are retained unchanged; ambiguous legacy matches
HOLD rather than being guessed or duplicated.

A null destination means the indexer has no active mapping, **not** that it has
proved a manager rejection. It stays a liability with authenticated Ethereum
source coordinates and no terminal completion. A later exact authenticated
manager settlement can supply the actual destination. Current manager
`ReceiptDeposits` token/supply Options and `Rejected(UnmappedToken)` must never be
converted to a zero token, invented supply, settlement or refund. The indexer does
not project these query outcomes as economic effects: settlement/refund events
still carry concrete token IDs, and an unmapped rejection emits neither event.
The manager's retained rejected liability and cutover gate remain authoritative.

## Finality and cumulative roots

The native `relayer ethereum-evidence` child remains the only Ethereum consensus
and receipt-proof authority. Its durable, revalidated ancestry chunks can survive
an interrupted long-gap descent; this does not advance SQL projection state.
Retain native proof stores, original SQL checkpoints and bootstrap approvals
across restart. Retained SQL checkpoints are read by immutable ID in pages of 512;
every row identity and every fence is qualified, including late-page conflicts.
Do not treat a stored verification flag or a successful earlier page as authority.

`MerkleRootInMessageQueue` records only actual finalized root publication events
at their emitted source block and Ethereum inclusion. A cumulative root can cover
earlier messages, but those earlier roots are not individually registered. The
indexer does not manufacture root rows from publisher `covered` inventory.
Outbound completion still requires the exact original message hash/nonce and
matching economic effect, not root availability alone. QueueId-aware proof
selection belongs to the relayer/SDK; an absent QueueId is never inferred as zero.

## Reconstructed regression coverage

Tests labeled `HTML7fe1b032 reconstructed` are reconstructed from the supplied
HTML audit of `7fe1b032dff5143df41b1df22ecaf23c4e2f7cb6`, not the unavailable
original PoC bundle. They exercise the candidate's built indexer and real native
fixture verifier. Release evidence must bind the actual candidate commit and
built-artifact digests; these tests alone do not establish production readiness.

After building the indexer and matching native evidence executables, run:

```sh
yarn workspace indexer test
yarn workspace indexer test:evidence-db
```

The evidence tests require `ETH_EVIDENCE_BINARY` and
`ETH_EVIDENCE_FIXTURE_BINARY`. The database suite additionally requires an
explicitly owned empty loopback PostgreSQL database via
`INDEXER_TEST_DATABASE_URL`; it refuses to repurpose an existing database.
The SQL suite exercises role/effect/checkpoint rollback after a callback save,
committed-state reload, database write failure propagation and unmapped-liability
migration rollback refusal. Native restart tests interrupt the real child after
a retained ancestry chunk is written, then deny fetching the chunk's interior
headers during restart. No test result is implied by this document.
