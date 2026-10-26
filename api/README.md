# Ethereum smart contracts ABI and gear programs idl files

Contents of this folder are guaranteed to be up to date as there's a workflow in CI that checks that they match with ones generated when the project is built.

> [!WARNING]
> Update CI workflow when adding new files to this folder

Regenerate the Ethereum snapshots from the repository root with the CI-pinned
Foundry v1.8.5 and the compiler/settings in `ethereum/foundry.toml` (Solc 0.8.37):

```sh
forge build --force --no-cache --root ethereum
for contract in IBridgingPayment IERC20Manager IMessageQueue WrappedEther ERC20Manager MessageQueue BridgingPayment BeefyClient VaraQueueRootVerifier; do
  cp "ethereum/out/$contract.sol/$contract.json" "api/ethereum/$contract.json"
done
```

Commit the complete generated artifacts, not just their ABI fields. Solidity AST
IDs and source coordinates depend on the full compilation input, so changes to
tests or scripts can require refreshing otherwise unchanged contract snapshots.
CI rebuilds the same complete input and compares every snapshot byte-for-byte.

Active Gear programs must use the reviewed Sails revision
`3cd30d98a22ad92f0c3ac3a1bf0c2fb360fb75c9` in both the workspace lock and every
per-program lock. Native CI checks these locks before building artifacts.
Resolve dependency changes through Cargo, not lockfile hash replacement.
Immutable historical-token fixture and provenance locks are excluded and must
remain byte-for-byte unchanged.

## Frontend Sails clients

After the locked Cargo build, export the fresh `target/wasm32-gear/release/*.idl`
files to their corresponding `api/gear/` paths listed in native CI. Do not replace
the immutable `vft_vara_v1.idl` fixture. With the existing workspace dependencies
installed, run from the repository root:

```sh
yarn workspace frontend generate:sails
yarn workspace frontend check:sails
```

The first command regenerates all seven canonical frontend clients; the second
compares fresh output byte-for-byte without changing repository files. Each uses
the installed `sails-js-cli` **0.5.0** command
`sails-js generate <Cargo-generated.idl> --no-project --out <owned-scratch-dir> --yes`
and removes only its own temporary directory. Generation completes for all clients
before writing outputs. No formatter or network access is required.

`js/frontend/generate-sails.mjs` preserves the existing `SailsProgram` exports,
service names, callback types and inline exported IDL types. It maps the Electra
IDL to the frontend's existing `eth-events.ts`; the Deneb and immutable-v1 IDLs
are not substitutes for that client. Event adaptation reuses the SDK's
`subscribeAdvisory`: malformed prefixes/bodies, noncanonical or trailing SCALE,
synchronous callback errors and rejected promises stop/report the subscription;
dispatch after unsubscribe is ignored. Events remain advisory, never settlement
evidence. Change the adaptation script, not generated event methods.

Native CI compares clients only after building and checking canonical Cargo IDLs;
JavaScript CI also compares checked-in IDLs so frontend-only drift cannot bypass
the check. Commit regenerated clients with source/IDL changes. The SDK clients in
`js/bridge-js/src/vara` contain deliberate handwritten adapters and are **not**
overwritten by this command; migrate and qualify their interfaces separately.