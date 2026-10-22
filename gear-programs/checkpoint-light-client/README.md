## The **checkpoint-light-client** program

The program workspace includes the following packages:
- `checkpoint-light-client` is the package allowing to build WASM binary for the program and IDL file for it. 
- `checkpoint-light-client-app` is the package containing business logic for the program represented by the `CheckpointLightClientService` structure.  
- `checkpoint-light-client-client` is the package containing the client for the program allowing to interact with it from another program, tests, or
  off-chain client.

## Owned isolated consensus fixture

`Network::LocalFixture { genesis_time, genesis_validators_root }` is an explicit,
immutable deployment identity appended after the four public network variants.
The public variants retain their SCALE discriminants and network constants;
unknown genesis roots are never automatically selected as fixtures.

Use `checkpoints-tool --owned-fixture-genesis-time <approved-time>` only with
loopback Gear (`ws`/`wss`) and Beacon (`http`/`https`) endpoints. Both
`--trusted-genesis-validators-root` and `--trusted-bootstrap-root` remain required
independent approvals. The Beacon response must match the approved genesis root
and time; public-network roots cannot be relabeled as fixtures. Retain the
reviewed genesis/config artifacts and approve the fresh checkpoint CodeId/IDL
and application profile separately; do not modify existing public approvals.

The fixture uses the standard mainnet preset: 512 sync-committee entries, 32
slots per epoch, 256 epochs per committee period and 12 seconds per slot. Deneb
and Electra are active at epoch zero, the Electra signing version is
`0x60003137`, and Fulu is disabled (`u64::MAX`). Real validators must produce
blocks, attestations, aggregate BLS signatures and finalized updates. Bootstrap
committee SSZ proofs, the signing domain, finality/next-committee proofs, voting
threshold and receipt verification are unchanged.

Initialization still requires a finalized update in committee period one or
later and a bootstrap header from that same period, with the authenticated next
committee proof. A genesis header or empty skipped slots alone cannot satisfy
this. A genesis time declared in the past may allow normal consensus processing
of skipped slots, but use a slot-zero genesis state and obtain actual subsequent
signed blocks and finality; never prefill fabricated headers or checkpoints.
Client support/readiness for this startup must be proven on the owned fixture.

Checkpoint initialization rejects preflight panics, then uses
the same 95% block-gas ceiling as the checkpoint worker. An exact minimum
estimate can underfund asynchronous builtin replies; demand above the ceiling
is rejected before signing. A finalized failed constructor retains its original
identity: authenticate its terminal reply before creating a distinct program.
