### governance-tool

Utility for generating unsigned payloads for Ethereum contracts. It reads the configured handlers and prints the message; it does not broadcast transactions or grant execution approval.

Governance handler payloads now use `0xff || abi.encode(uint8(1), uint64(sequence),
uint64(validUntil), uint8(action), address(target), uint64(incidentEpoch),
bytes32(requiredAdminAction), bytes32(requiredBarrier), bytes(innerPayload))`.
Pass independently approved `--sequence` and `--valid-until` **before the subcommand**
in every GovernanceAdmin/GovernancePauser example below. Sequences are per exact
source/destination stream; expiry is a positive inclusive Unix timestamp. They
are never inferred from RPC state. ERC20Manager registration messages are not
governance-handler controls and do not take these flags.

Unpause also requires `--incident-epoch` and `--required-admin-action` (the full
message hash of a successfully completed same-target, same-incident admin
control). A pending upgrade hash does not qualify. The first post-migration
queue unpause must name the exact migration message and `--required-barrier`.
Non-unpause controls require zero dependency fields. Admin `Cancel SOURCE
DESTINATION THROUGH` cancels an exact currently configured stream, including the
pauser stream; retired sources cannot be rotated back into authority.

Retained migration preparation must prepublish and mature the exact legacy
migration and recovery upgrade messages before installing the staging
implementation. `stageLegacyMigration(expectedVerifier, C, migration, recovery)`
(initializer 7) checks their full message identities, membership, maturity,
expiry, calldata and exact current maximum C while paused. Both publication
paths then freeze. Only these two exact bounded legacy exceptions remain.
`migrateToBeefy` (initializer 8) consumes the stage atomically; recovery before
migration clears the stage while preserving retained verifier/claims. A recovered
initializer-7 deployment needs a separately reviewed/approved restaging path,
not replay of the consumed initializer.

After a real authenticated post-migration snapshot, anyone may call
`captureMigrationPublicationHash(sourceBlock)` within the 256-block `blockhash`
window. This stores the actual canonical hash under the complete authenticated
publication identity (including QueueId/root/source time/publication block); it
does **not** attest finality. `migrationPublicationBlockHash(sourceBlock)` exposes
that capture only for the currently retained publication.

The approved admin subsequently calls `recordMigrationBarrier(sourceBlock,
publicationBlockHash)` through a versioned upgrade-handler control, after
independently qualifying destination finality. It checks the retained capture,
so the one-hour admin maturity need not fit the blockhash window. **The admin,
not the EVM or the capture caller, attests finality.** Empty and nonempty progress
may use the same QueueId; source time and destination publication must be after
migration. An uncaptured publication older than 256 blocks requires a newer
authenticated snapshot; neither a caller-supplied hash nor an RPC claim repairs it.
Unused initializer/storage-slot approval, source coverage, operator finality
attestation and staging recovery availability remain release gates.

```bash
# Fill `deployment.toml` with addresses from mainnet/testnet!
cp deployment.example.mainnet.toml deployment.toml
cp deployment.example.sepolia.toml deployment.toml
cp deployment.example.hoodi.toml deployment.toml

MAINNET_RPC_URL="https://ethereum-rpc.publicnode.com"
SEPOLIA_RPC_URL="https://ethereum-sepolia-rpc.publicnode.com"
HOODI_RPC_URL="https://ethereum-hoodi-rpc.publicnode.com"

# Select one of RPCs
RPC_URL=$MAINNET_RPC_URL
RPC_URL=$SEPOLIA_RPC_URL
RPC_URL=$HOODI_RPC_URL

cargo run --package governance-tool --release -- --ethereum-endpoint $RPC_URL --help
```

### Examples

These are alternative commands, not a batch. Set `SEQUENCE` and `VALID_UNTIL` from the independently approved message before each handler command; do not reuse a stream sequence for another action. For unpause, also set `INCIDENT_EPOCH` and `REQUIRED_ADMIN_ACTION`; queue unpause additionally uses the approved `REQUIRED_BARRIER` (zero only when no migration barrier is required).

`GovernanceAdmin` actions

#### Change governance (from GovernanceAdmin)

```bash
NEW_GOVERNANCE_RAW="0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin ChangeGovernance "$NEW_GOVERNANCE_RAW"

NEW_GOVERNANCE_VARA="kGkLEU3e3XXkJp2WK4eNpVmSab5xUNL9QtmLPh8QfCL2EgotW" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin ChangeGovernance "$NEW_GOVERNANCE_VARA"
```

#### Pause proxy (from GovernanceAdmin)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin PauseProxy "$PROXY_WVARA"

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin PauseProxy "$PROXY_MQ"

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin PauseProxy "$PROXY_ERC20MNGR"
```

#### Unpause proxy (from GovernanceAdmin)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" --incident-epoch "$INCIDENT_EPOCH" --required-admin-action "$REQUIRED_ADMIN_ACTION" GovernanceAdmin UnpauseProxy "$PROXY_WVARA"

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" --incident-epoch "$INCIDENT_EPOCH" --required-admin-action "$REQUIRED_ADMIN_ACTION" --required-barrier "$REQUIRED_BARRIER" GovernanceAdmin UnpauseProxy "$PROXY_MQ"

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" --incident-epoch "$INCIDENT_EPOCH" --required-admin-action "$REQUIRED_ADMIN_ACTION" GovernanceAdmin UnpauseProxy "$PROXY_ERC20MNGR"
```

#### Upgrade proxy (from GovernanceAdmin)

```bash
PROXY_WVARA="WrappedVara"
NEW_IMPLEMENTATION="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin UpgradeProxy "$PROXY_WVARA" "$NEW_IMPLEMENTATION" 0x
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin UpgradeProxy "$PROXY_WVARA" "$NEW_IMPLEMENTATION" "$(cast calldata 'function reinitialize()')"

PROXY_MQ="MessageQueue"
NEW_IMPLEMENTATION="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin UpgradeProxy "$PROXY_MQ" "$NEW_IMPLEMENTATION" 0x
MIGRATION_DATA=$(cast calldata "migrateToBeefy(address,address,uint256,uint256,address)" "$LEGACY_VERIFIER" "$BEEFY_VERIFIER" "$LEGACY_MAX_BLOCK" "$FIRST_BEEFY_BLOCK" "$BEEFY_RECOVERY_TIMELOCK")
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin UpgradeProxy "$PROXY_MQ" "$NEW_IMPLEMENTATION" "$MIGRATION_DATA"

PROXY_ERC20MNGR="ERC20Manager"
NEW_IMPLEMENTATION="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin UpgradeProxy "$PROXY_ERC20MNGR" "$NEW_IMPLEMENTATION" 0x
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernanceAdmin UpgradeProxy "$PROXY_ERC20MNGR" "$NEW_IMPLEMENTATION" "$(cast calldata 'function reinitialize()')"
```

For an existing ZK queue, use the migration payload only after the [mainnet release gates](../../ethereum/README.md#mainnet-custody-preserving-migration-candidate), with the queue paused and the independent recovery timelock's code, multisig roles and positive delay approved. Empty upgrade data is not a ZK-to-BEEFY verifier cutover. Payload generation does not authorize governance execution.

---

`GovernancePauser` actions

#### Change governance (from GovernancePauser)

```bash
NEW_GOVERNANCE_RAW="0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernancePauser ChangeGovernance "$NEW_GOVERNANCE_RAW"

NEW_GOVERNANCE_VARA="kGkLEU3e3XXkJp2WK4eNpVmSab5xUNL9QtmLPh8QfCL2EgotW" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernancePauser ChangeGovernance "$NEW_GOVERNANCE_VARA"
```

#### Pause proxy (from GovernancePauser)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernancePauser PauseProxy "$PROXY_WVARA"

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernancePauser PauseProxy "$PROXY_MQ"

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" GovernancePauser PauseProxy "$PROXY_ERC20MNGR"
```

#### Unpause proxy (from GovernancePauser)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" --incident-epoch "$INCIDENT_EPOCH" --required-admin-action "$REQUIRED_ADMIN_ACTION" GovernancePauser UnpauseProxy "$PROXY_WVARA"

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" --incident-epoch "$INCIDENT_EPOCH" --required-admin-action "$REQUIRED_ADMIN_ACTION" --required-barrier "$REQUIRED_BARRIER" GovernancePauser UnpauseProxy "$PROXY_MQ"

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --ethereum-endpoint "$RPC_URL" --sequence "$SEQUENCE" --valid-until "$VALID_UNTIL" --incident-epoch "$INCIDENT_EPOCH" --required-admin-action "$REQUIRED_ADMIN_ACTION" GovernancePauser UnpauseProxy "$PROXY_ERC20MNGR"
```

---

`ERC20Manager` actions

#### Add new VFT manager (from GovernanceAdmin)

```bash
VFT_MANAGER_RAW="0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --ethereum-endpoint $RPC_URL ERC20Manager AddVftManager $VFT_MANAGER_RAW

VFT_MANAGER_VARA="kGkLEU3e3XXkJp2WK4eNpVmSab5xUNL9QtmLPh8QfCL2EgotW" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --ethereum-endpoint $RPC_URL ERC20Manager AddVftManager $VFT_MANAGER_VARA
```

#### Register Ethereum token (from GovernanceAdmin)

```bash
TOKEN="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --ethereum-endpoint $RPC_URL ERC20Manager RegisterEthereumToken $TOKEN
```

#### Register Gear token (from GovernanceAdmin)

```bash
TOKEN_NAME="My Token"
TOKEN_SYMBOL="MTK"
TOKEN_DECIMALS="18"
cargo run --package governance-tool --release -- --ethereum-endpoint $RPC_URL ERC20Manager RegisterGearToken $TOKEN_NAME $TOKEN_SYMBOL $TOKEN_DECIMALS
```
