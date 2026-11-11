# Bridge indexer configuration

The Gear processor requires explicit RPC, Subsquid gateway, start block and bridge
program IDs. Copy `.env.example` to `.env` in the repository root and fill it from
one verified deployment. Exported environment variables override `.env` values.
The checked-in deployment workflow delegates runtime configuration to external
Helm/ConfigMap/Secret values; verify the effective Pod environment before rollout.

Required network variables:

- `GEAR_RPC_URL`: the selected Substrate chain's RPC.
- `GEAR_ARCHIVE_URL`: its Subsquid HTTP gateway, covering the start block.
- `GEAR_FROM_BLOCK`: a non-negative integer at/before the first relevant bridge event.
- `GEAR_VFT_MANAGER`, `GEAR_HISTORICAL_PROXY`, `GEAR_BRIDGING_PAYMENT`,
  `GEAR_CHECKPOINT_CLIENT`: 32-byte program IDs from that deployment.
- `GEAR_SQUID_API_KEY`: gateway access key.
- `ETH_HTTP_RPC_URL`: execution RPC for the paired Ethereum chain.

Missing or malformed network values stop startup. Public program addresses alone
are insufficient to select the gateway and start block. Configure the paired
Ethereum processor's RPC, gateway, start block and contracts explicitly too.
Vara.eth/Hoodi is a separate environment and cannot replace a Substrate RPC.

For a same-network upgrade, retain the database and `GEAR_STATE_SCHEMA` (default
`gear_processor`) so processing resumes from its existing cursor. Changing
`GEAR_FROM_BLOCK` does not rewind that cursor. A new network or reindex requires a
separate persistent database for both processors and their transfer/pair/program
state; a different cursor schema alone does not isolate networks. Point
`DATABASE_URL` and `DB_*` to the same database and keep the old database for rollback.

From the repository root, run `yarn build:indexer`, `yarn lint:indexer` and
`yarn workspace indexer test:config`. After configuring the database, run
`yarn workspace indexer migration:run` with `DATABASE_URL` exported, then
`node js/indexer/lib/gear/main.js` from the root so API paths resolve.

Before deployment, verify genesis, gateway coverage, program IDs, first event
height and paired Ethereum contracts. On staging, confirm initial progress and
archive-to-RPC handoff; stop cleanly and restart with the same database/schema.
Check persisted cursor advancement and absence of missing/duplicate transfers.
The indexer image also serves the ETH processor and API: roll out or roll back
image and configuration together. These live checks require deployment access.
