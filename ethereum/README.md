## Foundry

**Foundry is a blazing fast, portable and modular toolkit for Ethereum application development written in Rust.**

Foundry consists of:

- **Forge**: Ethereum testing framework (like Truffle, Hardhat and DappTools).
- **Cast**: Swiss army knife for interacting with EVM smart contracts, sending transactions and getting chain data.
- **Anvil**: Local Ethereum node, akin to Ganache, Hardhat Network.
- **Chisel**: Fast, utilitarian, and verbose solidity REPL.

## Documentation

https://book.getfoundry.sh/

## Usage

### Install dependencies

```shell
$ forge soldeer install
```

### Build

```shell
$ forge build
```

### Test

```shell
$ forge test
```

### Format

```shell
$ forge fmt
```

### Gas Snapshots

```shell
$ forge snapshot
```

### Anvil

```shell
$ anvil
```

### Deploy

```shell
# deployment
$ cp .env.example .env
# fork testing on mainnet
$ cp .env.example.mainnet .env
# fork testing on sepolia
$ cp .env.example.sepolia .env
# fork testing on hoodi
$ cp .env.example.hoodi .env

$ source .env

$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/Deployment.s.sol:DeploymentScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

### BEEFY deployment and regression checks

`BeefyLocal`, `BeefyHoodi`, and `BeefyTokens` share queue CREATE predictions in
`test/Base.sol`, beside the deployment order: the client consumes one nonce,
token profiles consume seven more, and governance/verifier/queue implementation
consume four before the queue proxy. The original queue offsets remain `+12`
for token profiles and `+5` for message-only Hoodi. Token overrides remain
forbidden for these predictions; `BeefyTokens` also checks the expected starting
nonce before deploying. The configured bridge domain must match the predicted
queue and chain, and the client/verifier bind that destination immutably.

The interactive and Fiat-Shamir submissions share authenticated leaf acceptance,
authority rotation, and ECDSA verification. Ticket handling, randomness, sampling,
proof-check ordering, signature counts, and usage counters remain protocol-specific.

Run the focused checks with installed dependencies/compiler caches:

```shell
$ forge test --offline --match-path test/BeefyInterop.t.sol
$ forge test --offline --match-path test/BeefySubmissionParity.t.sol
$ forge test --offline --match-path test/VaraBeefyQueue.t.sol
$ forge test --offline --threads 1 --match-path test/script/Deployment.t.sol --match-test 'Beefy|Hoodi'
```

Deployment tests mutate process-wide environment variables, so run them with one
thread. If OpenZeppelin validation reports duplicate contracts in `out/build-info`,
use a fresh task-owned `FOUNDRY_OUT` and `FOUNDRY_CACHE_PATH` for that check rather
than deleting existing deployment/build evidence. A forced no-cache build followed
by selectively compiled tests can also leave duplicate build-info batches; run
the focused suite in one fresh output directory. Keep script smoke outputs
separate from test outputs to avoid combining incompatible build-info batches.

For a deployment dry-run, supply fresh bootstrap/domain environment values and run
`forge script --offline script/BeefyLocal.s.sol:BeefyLocal --chain-id 31337`
(or the matching Hoodi/token script and chain). Omit `--broadcast`; simulation
alone sends no transactions. Never retry a partial broadcast without reconciling
receipts, deployed contracts, and the deployer nonce.

These scripts deploy fresh, isolated BEEFY lanes; they do not migrate an existing
ZK queue proxy. Use the gated existing-proxy migration below instead.

### Mainnet custody-preserving migration candidate

Keep the existing queue proxy, ERC20 manager, token contracts and Vara programs.
The queue at `0x8E01Fbf136cA97627ca241dB9EFf1DFE3f2195F6` used implementation
`0xadf9f599277f41907cf86109833d1601b433dc5b` (initializer version 6) at Ethereum
block 26,183,239. Its authenticated runtime bytecode and real claim inputs are in
`test/fixtures/mainnet-legacy-queue.json`; compilation used Solc 0.8.35, with only
compiler-declared implementation-address immutables substituted for comparison.

The candidate exposes `migrateToBeefy(expectedLegacyVerifier, beefyVerifier,
expectedLegacyMaxBlock, firstBeefyBlock)` as an admin-only `reinitializer(8)`.
Execute it atomically through the existing authority's `upgradeToAndCall` while
paused. It preserves roles, custody, genesis, maximum progress, registered roots,
legacy root-keyed maturity and every replay bit, including blocked nonce 836.
It does not grant a replacement administrator. The expected verifier and maximum
are compare-and-swap guards: root publication is not stopped by pausing claims.

The new floor must exceed the legacy maximum, be at or after MMR history start,
fit uint32 and remain within the existing 57,600-block progress bound. The
queue-bound, policy-compatible BEEFY client must be live and already have an
authenticated nonzero MMR root covering the floor. No old ZK proof is accepted
after migration. Previously registered roots remain redeemable below the floor;
new registrations use block-specific maturity. Reconcile and publish all needed
legacy root coverage before selecting the floor; catch up with genuine ZK roots
if the MMR start is beyond the distance bound. Never invent empty progress.

Run the bytecode transition regressions and the opt-in archival fork rehearsal:

```shell
forge test --match-contract VaraQueueRootVerifierTest --match-test testLegacyMigration
MAINNET_MIGRATION_RPC_URL="$ARCHIVE_RPC" forge test --match-contract VaraQueueRootVerifierTest --match-test testMainnetForkMigration
```

The fork pins block hash
`0x8e462035baf49ef0bdd7ede06fb133e6aca0f34c25833387d3c87500947a7a3d`, checks deployed
code/version, restricts queue migration writes to the implementation, initializer,
verifier and floor slots, preserves existing custody/supply and redeems authentic
pending claim 21 with its retained proof. It preserves replay bits 836 and 864 and
rejects repeated redemption. It uses local authority impersonation and fixture
BEEFY authorities, not an approved mainnet bootstrap or a source governance proof.
Without the RPC variable, the fork test is explicitly skipped, not qualified.

Release pipeline, in order:

1. Authenticate the exact legacy and candidate compiler inputs/artifacts and run
   OpenZeppelin storage validation, including namespaced base storage. Preserve
   the working branch and extract a reviewed migration candidate; do not merge
   the whole architectural WIP as a release shortcut.
2. Approve the production source runtime, weights/full-state migration rehearsal,
   validator BEEFY keys, destination-bound domain and activation/MMR insertion pins.
   At the 2026-10-13 read-only refresh, both public Vara endpoints reported spec
   11000 at block 36,889,688, without BEEFY/MMR runtime APIs or BEEFY fields in
   queued session keys. These are matching provider observations, not authenticated
   source finality or proof of production activation.
   Never wire destructive queue/nonce reset migration. Retain the original queue.
3. Qualify mainnet follower network admission, Ethereum execution/Beacon finality
   and immutable bootstrap evidence. Follower, root publisher and history audit now
   accept explicit `--ethereum-network mainnet` with a reviewed `mainnet-token-stack`
   manifest, chain 1 and the fixed Ethereum mainnet genesis. Hoodi remains the default;
   mainnet rejects fixture/rehearsal approval and non-TLS execution endpoints.
   Network support is not approval of production bootstrap or Beacon/source state.
   Preserve original journals; exercise dropped-original recovery, external progress
   and backfill against the approved retained-state profile.
   The normal token follower requires the queue to be bound to the new verifier,
   while migration requires an accepted nonzero client MMR root beyond the cut floor.
   Separately qualify the original bootstrap commitment before the atomic queue
   cutover; the normal follower is not a pre-migration binding-check bypass.
4. Reconcile all five assets, all historical managers/native programs, paid and
   unpaid pending messages, registered and missing roots, fees and replay state
   through an approved cut. Token identity agreement and sample claims are not
   complete liability reconciliation. Keeping old Vara managers does not apply
   this branch's newer manager code to those immutable programs.
5. Exercise both transfer directions, SDK/frontend settlement and HOLD behavior,
   both indexers, source governance authorization and interruption/restart against
   the exact retained-state release profile. A fresh-token deployment is not this
   acceptance test. Retain legacy claim services and historical proofs.
6. Review implementation/verifier/client hashes, governance authority/policy,
   expected legacy verifier/maximum, floor, calldata and pause/unpause sequencing.
   Only separately authorized production governance may execute the upgrade.
   Unpause after post-upgrade state checks; rehearse recovery without undoing any
   accepted BEEFY progress or processed claims.

Source activation, mainnet follower qualification, full accounting, end-to-end
acceptance and governance approval remain release gates. Local fork success
neither authorizes a public transaction nor qualifies mainnet activation.


### Copied BEEFY sources and licenses

`src/beefy/BeefyClient.sol` and the Snowfork utilities retain their upstream
copyright and SPDX headers. The interoperability source pin is Snowbridge
[`1201293e482ef052b9c3989dcf680046704fef3d`](https://github.com/Snowfork/snowbridge/tree/1201293e482ef052b9c3989dcf680046704fef3d);
the architectural reference is
[`630c2081359e2417496e7f4c1a189a7eb40f7bd4`](https://github.com/Snowfork/snowbridge/tree/630c2081359e2417496e7f4c1a189a7eb40f7bd4).
The local client and utilities are modified for Vara; no upstream audit is
transferred to this code or its deployment bytecode.

- [Apache-2.0 text](src/beefy/LICENSE-APACHE-2.0): verbatim Snowbridge
  `contracts/LICENSE` at the interoperability pin (identical at the architectural
  reference). Neither pinned upstream tree includes a `NOTICE` file.
- [MIT text](src/beefy/LICENSE-MIT): verbatim OpenZeppelin Contracts
  [`v5.0.0/LICENSE`](https://github.com/OpenZeppelin/openzeppelin-contracts/blob/v5.0.0/LICENSE),
  preserving its 2016–2023 copyright notice. `utils/Math.sol` also preserves
  the OpenZeppelin and Snowfork copyright notices from the copied source.
- [Original Bits MIT text](src/beefy/LICENSE-MIT-solidity-examples): verbatim
  [`ethereum/solidity-examples/LICENSE`](https://github.com/ethereum/solidity-examples/blob/ba15d56b62cf384e983006bb139af59a03ce22dc/LICENSE)
  at `ba15d56b62cf384e983006bb139af59a03ce22dc`, the upstream credited by
  `utils/Bits.sol`; the original copyright line is preserved as supplied.


### Relaying

```shell
$ source .env

$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/ProcessMessage.s.sol:ProcessMessageScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/relayers/SubmitMerkleRoot.s.sol:SubmitMerkleRootScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

### Upgrade

> [!WARNING]
> Existing ZK queues use the reviewed `migrateToBeefy` calldata below, never a fresh initializer or an empty-data verifier cutover. Other proxy upgrades require their own reviewed initialization and storage validation.

```shell
$ source .env

$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/ERC20Manager.s.sol:ERC20ManagerScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/MessageQueue.s.sol:MessageQueueScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMock.s.sol:VerifierMockScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierMainnet.s.sol:VerifierMainnetScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/VerifierTestnet.s.sol:VerifierTestnetScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv

$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $LOCAL_RPC_URL --broadcast -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $MAINNET_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $SEPOLIA_RPC_URL --broadcast --verify -vvvv
$ forge script script/upgrades/WrappedVara.s.sol:WrappedVaraScript --rpc-url $HOODI_RPC_URL --broadcast --verify -vvvv
```

### Preparing the ZK-to-BEEFY governance payload

After satisfying the mainnet migration gates above, use independently reviewed
addresses and cut heights to generate calldata; this does not authorize execution.

```bash
MIGRATION_DATA=$(cast calldata "migrateToBeefy(address,address,uint256,uint256)" "$LEGACY_VERIFIER" "$BEEFY_VERIFIER" "$LEGACY_MAX_BLOCK" "$FIRST_BEEFY_BLOCK")
cargo run --package governance-tool --release -- --ethereum-endpoint "$MAINNET_RPC_URL" GovernanceAdmin UpgradeProxy MessageQueue "$NEW_IMPLEMENTATION" "$MIGRATION_DATA"
```

The queue must already be paused. Execute only via the approved existing source
governance authority and retain the original message, proof and transaction identity.
Do not retry an uncertain governance message before reconciling its original outcome.

### Coverage

We use Docker image and slightly modified sources to produce coverage report in HTML format.

```shell
$ docker build --tag gear-bridges/ethereum-contracts .
$ docker run --rm -it --volume "$(pwd)":/files gear-bridges/ethereum-contracts

$ sed -i '/if (!BinaryMerkleTree.verifyProofCalldata(merkleRoot, proof, totalLeaves, leafIndex, messageHash)) {/{N;N;d;}' src/MessageQueue.sol
$ sed -i '/emit MessageProcessed(blockNumber, messageHash, message.nonce, message.destination);/d' src/MessageQueue.sol
$ sed -i '/function test_ProcessMessageWithInvalidMerkleProof()/,/^[[:space:]]*}[[:space:]]*$/d' test/MessageQueue.t.sol
$ forge test
$ forge coverage --report lcov
$ genhtml lcov.info --branch-coverage --output-dir /files/coverage
$ exit

$ docker image rm gear-bridges/ethereum-contracts
$ docker buildx prune --all

$ sudo chown -R $(whoami):$(whoami) coverage
$ firefox coverage/index.html
```

### Cast

```shell
$ cast <subcommand>
```

### Help

```shell
$ forge --help
$ anvil --help
$ cast --help
```
