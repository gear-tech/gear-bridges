### governance-tool

Utility for generating payloads for Ethereum contracts

```bash
cp deployment.example.mainnet.toml deployment.toml
# Fill `deployment.toml` with addresses from mainnet!

MAINNET_RPC_URL="https://ethereum-rpc.publicnode.com"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL --help
```

### Examples

`GovernanceAdmin` actions

#### Change governance (from GovernanceAdmin)

```bash
NEW_GOVERNANCE_RAW="0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin ChangeGovernance $NEW_GOVERNANCE_RAW

NEW_GOVERNANCE_VARA="kGkLEU3e3XXkJp2WK4eNpVmSab5xUNL9QtmLPh8QfCL2EgotW" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin ChangeGovernance $NEW_GOVERNANCE_VARA
```

#### Pause proxy (from GovernanceAdmin)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin PauseProxy $PROXY_WVARA

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin PauseProxy $PROXY_MQ

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin PauseProxy $PROXY_ERC20MNGR
```

#### Unpause proxy (from GovernanceAdmin)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UnpauseProxy $PROXY_WVARA

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UnpauseProxy $PROXY_MQ

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UnpauseProxy $PROXY_ERC20MNGR
```

#### Upgrade proxy (from GovernanceAdmin)

```bash
PROXY_WVARA="WrappedVara"
NEW_IMPLEMENTATION="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UpgradeProxy $PROXY_WVARA $NEW_IMPLEMENTATION 0x
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UpgradeProxy $PROXY_WVARA $NEW_IMPLEMENTATION $(cast calldata "function reinitialize()")

PROXY_MQ="MessageQueue"
NEW_IMPLEMENTATION="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UpgradeProxy $PROXY_MQ $NEW_IMPLEMENTATION 0x
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UpgradeProxy $PROXY_MQ $NEW_IMPLEMENTATION $(cast calldata "function reinitialize()")

PROXY_ERC20MNGR="ERC20Manager"
NEW_IMPLEMENTATION="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UpgradeProxy $PROXY_ERC20MNGR $NEW_IMPLEMENTATION 0x
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernanceAdmin UpgradeProxy $PROXY_ERC20MNGR $NEW_IMPLEMENTATION $(cast calldata "function reinitialize()")
```

---

`GovernancePauser` actions

#### Change governance (from GovernancePauser)

```bash
NEW_GOVERNANCE_RAW="0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser ChangeGovernance $NEW_GOVERNANCE_RAW

NEW_GOVERNANCE_VARA="kGkLEU3e3XXkJp2WK4eNpVmSab5xUNL9QtmLPh8QfCL2EgotW" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser ChangeGovernance $NEW_GOVERNANCE_VARA
```

#### Pause proxy (from GovernancePauser)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser PauseProxy $PROXY_WVARA

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser PauseProxy $PROXY_MQ

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser PauseProxy $PROXY_ERC20MNGR
```

#### Unpause proxy (from GovernancePauser)

```bash
PROXY_WVARA="WrappedVara"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser UnpauseProxy $PROXY_WVARA

PROXY_MQ="MessageQueue"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser UnpauseProxy $PROXY_MQ

PROXY_ERC20MNGR="ERC20Manager"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL GovernancePauser UnpauseProxy $PROXY_ERC20MNGR
```

---

`ERC20Manager` actions

#### Add new VFT manager (from GovernanceAdmin)

```bash
VFT_MANAGER_RAW="0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL ERC20Manager AddVftManager $VFT_MANAGER_RAW

VFT_MANAGER_VARA="kGkLEU3e3XXkJp2WK4eNpVmSab5xUNL9QtmLPh8QfCL2EgotW" # https://ss58.org (any address format from this website is supported)
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL ERC20Manager AddVftManager $VFT_MANAGER_VARA
```

#### Register Ethereum token (from GovernanceAdmin)

```bash
TOKEN="0x0000000000000000000000000000000000000000" # must exist on https://etherscan.io
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL ERC20Manager RegisterEthereumToken $TOKEN
```

#### Register Gear token (from GovernanceAdmin)

```bash
TOKEN_NAME="My Token"
TOKEN_SYMBOL="MTK"
TOKEN_DECIMALS="18"
cargo run --package governance-tool --release -- --rpc-url $MAINNET_RPC_URL ERC20Manager RegisterGearToken $TOKEN_NAME $TOKEN_SYMBOL $TOKEN_DECIMALS
```
