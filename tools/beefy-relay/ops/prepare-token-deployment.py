#!/usr/bin/env python3
"""One attempt at a fresh, test-only Hoodi BeefyTokens deployment. No replay path."""

import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat
import subprocess
from urllib.request import Request, urlopen

from eth_account import Account
from eth_utils import keccak, to_checksum_address
import rlp


from run_context import RUN, RPC, MANIFEST, NETWORK, checked_file
PROJECT = RUN / "forge-final"
CHAIN_ID = 560048
HOODI_GENESIS = "0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b"
SCRIPT_SHA256 = MANIFEST["solidity"]["scriptSha256"]
ARTIFACT_SHA256 = MANIFEST["solidity"]["artifactSha256"]
FEE_WEI = 1_000_000_000_000
ZERO = "0x" + "00" * 20


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def load(path):
    return json.loads(path.read_text())


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def hex_bytes(value, size, label):
    require(isinstance(value, str) and value.startswith("0x") and len(value) == 2 + 2 * size, f"invalid {label}")
    try:
        result = bytes.fromhex(value[2:])
    except ValueError as error:
        raise RuntimeError(f"invalid {label}") from error
    require(any(result), f"zero {label}")
    return result


def uint(value, bits, label):
    require(type(value) is int and 0 <= value < (1 << bits), f"invalid {label}")
    return value


def key(path):
    info = path.lstat()
    require(stat.S_ISREG(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o600, f"private key must be a regular mode-0600 file: {path.name}")
    value = load(path)
    private_key = hex_bytes(value["private_key"], 32, path.name)
    address = Account.from_key(private_key).address
    require(address.lower() == value["address"].lower(), f"private-key address mismatch: {path.name}")
    return value["private_key"], address


def rpc(method, params, endpoint=RPC):
    data = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}).encode()
    request = Request(endpoint, data=data, headers={"Content-Type": "application/json", "User-Agent": "Mozilla/5.0 (test-only-beefy-deployment)"})
    with urlopen(request, timeout=30) as response:
        result = json.load(response)
    require("error" not in result and result.get("id") == 1 and result.get("result") is not None, f"Hoodi RPC {method} failed")
    return result["result"]


def gear_rpc(endpoint, method, *params):
    require(endpoint.startswith("ws://127.0.0.1:"), "source must use a loopback-only Gear RPC")
    command = ["rtk", "cast", "rpc", method, *(str(param) for param in params), "--rpc-url", endpoint, "--rpc-timeout", "8"]
    try:
        answer = subprocess.check_output(command, stderr=subprocess.PIPE, timeout=12)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        raise RuntimeError(f"Gear witness {method} unavailable; hold deployment") from error
    return json.loads(answer)


def qualified_project_file(relative):
    checked_file("ethereum/" + relative)
    expected = MANIFEST["files"]["ethereum/" + relative]
    path = PROJECT / relative
    require(path.is_file() and not path.is_symlink() and digest(path) == expected,
            "Qualified Solidity project file changed or missing: " + relative)
    return expected


def compiled_artifact():
    foundry_files = {}
    for name in ("foundry.toml", "foundry.lock", "remappings.txt", "cache/solidity-files-cache.json"):
        if "ethereum/" + name in MANIFEST["files"]:
            foundry_files[name] = qualified_project_file(name)
        else:
            path = PROJECT / name
            require(name not in ("foundry.toml", "cache/solidity-files-cache.json") and not path.exists() and not path.is_symlink(),
                    "Unqualified Foundry input: " + name)
            foundry_files[name] = None
    for name in ("src", "script", "test"):
        require((PROJECT / name).is_dir() and not (PROJECT / name).is_symlink(), f"{name} must be a local verified source snapshot")
    script = PROJECT / "script/BeefyTokens.s.sol"
    require(digest(script) == SCRIPT_SHA256, "hardened BeefyTokens script changed since build")
    artifact_path = PROJECT / "out/BeefyTokens.s.sol/BeefyTokens.json"
    require(digest(artifact_path) == ARTIFACT_SHA256, "BeefyTokens artifact changed since build")
    artifact = load(artifact_path)
    metadata = artifact["metadata"]
    require(metadata["settings"]["compilationTarget"] == {"script/BeefyTokens.s.sol": "BeefyTokens"}, "wrong compilation target")
    require(metadata["compiler"]["version"].startswith("0.8.37+"), "wrong Solidity compiler")
    run = [item for item in artifact["abi"] if item.get("name") == "run" and item["type"] == "function"]
    require(len(run) == 1 and run[0]["inputs"] == [] and [x["type"] for x in run[0]["outputs"]] == ["address"] * 4, "BeefyTokens run ABI changed")
    build_infos = list((PROJECT / "out/build-info").glob("*.json"))
    require(len(build_infos) == 1, "BeefyTokens build-info ambiguous or missing")
    build_info_sha256 = qualified_project_file(str(build_infos[0].relative_to(PROJECT)))
    build = load(build_infos[0])
    sources = build["input"]["sources"]
    require(metadata["sources"].keys() <= sources.keys(), "compiled source set changed")
    for name, source in sources.items():
        require(not Path(name).is_absolute() and ".." not in Path(name).parts, "unsafe compiled source path")
        raw = (PROJECT / name).read_bytes()
        require(raw == source["content"].encode(), f"compiled source changed: {name}")
        if name in metadata["sources"]:
            require(keccak(raw).hex() == metadata["sources"][name]["keccak256"][2:], f"source metadata changed: {name}")
    result = build["output"]["contracts"]["script/BeefyTokens.s.sol"]["BeefyTokens"]
    require(artifact["abi"] == result["abi"] and artifact["bytecode"]["object"].removeprefix("0x") == result["evm"]["bytecode"]["object"], "compiled ABI/bytecode differs from build-info")
    contracts = {}
    for name, source in {
        "BeefyClient": "src/beefy/BeefyClient.sol",
        "VaraQueueRootVerifier": "src/VaraQueueRootVerifier.sol",
        "MessageQueue": "src/MessageQueue.sol",
        "ERC20Manager": "src/ERC20Manager.sol",
        "WrappedVara": "src/erc20/WrappedVara.sol",
        "ERC1967Proxy": "dependencies/@openzeppelin-contracts-5.7.0/proxy/ERC1967/ERC1967Proxy.sol",
    }.items():
        path = PROJECT / "out" / (name + ".sol") / (name + ".json")
        artifact_sha256 = qualified_project_file(str(path.relative_to(PROJECT)))
        compiled = load(path)
        expected = build["output"]["contracts"][source][name]
        require(compiled["metadata"]["settings"]["compilationTarget"] == {source: name}, f"wrong {name} compilation target")
        require(compiled["abi"] == expected["abi"] and all(compiled[key]["object"].removeprefix("0x") == expected["evm"][evm_key]["object"] for key, evm_key in (("bytecode", "bytecode"), ("deployedBytecode", "deployedBytecode"))), f"{name} interface or bytecode differs from hardened build")
        contracts[name] = {"artifactSha256": artifact_sha256, "creationCodeKeccak256": "0x" + keccak(hexstr=compiled["bytecode"]["object"]).hex(), "runtimeTemplateKeccak256": "0x" + keccak(hexstr=compiled["deployedBytecode"]["object"]).hex()}
    return {"scriptSha256": SCRIPT_SHA256, "artifactSha256": ARTIFACT_SHA256, "buildInfoSha256": build_info_sha256,
            "foundryFilesSha256": foundry_files, "scriptCreationCodeKeccak256": "0x" + keccak(hexstr=artifact["bytecode"]["object"]).hex(), "contracts": contracts}


def source_inputs():
    network_gate_path = RUN / "hoodi/network-gate.json"
    network_gate = load(network_gate_path)
    require(network_gate["status"] == "VERIFIED FOR TEST-ONLY SETUP" and network_gate["chainId"] == CHAIN_ID and network_gate["genesisHash"].lower() == HOODI_GENESIS and network_gate["executionHttp"] == RPC and network_gate["executionWss"] == NETWORK["executionWss"], "Hoodi archive-capable network gate missing")
    identity_path = RUN / "source-chain/identity.json"
    identity = load(identity_path)
    marker_path = RUN / "hoodi/funding-complete.json"
    funding = load(marker_path)
    spec = load(RUN / "source-chain/spec-evidence.json")
    launch = load(RUN / "source-chain/launch-state.json")
    stack_path = RUN / "token-stack/token-stack.json"
    stack = load(stack_path)
    anchor_path = RUN / "anchor.json"
    anchor = load(anchor_path)
    gear_roles = load(RUN / "hoodi/gear-addresses.json")["roles"]
    roles = load(RUN / "hoodi/addresses.json")
    require(identity["runId"] == RUN.name and spec["runId"] == RUN.name, "source identity belongs to a different run")
    require(identity["destinationChainId"] == spec["destinationChainId"] == CHAIN_ID, "wrong destination chain")
    require(identity["nonce"]["rpc"] in ("https://ethereum-hoodi-rpc.publicnode.com", RPC), "reservation RPC is not a pinned Hoodi endpoint")
    require(identity["funding"]["markerPath"] == str(marker_path) and identity["funding"]["phase"] == funding["phase"] == "finalized", "funding finality marker missing")
    require(funding["chainId"] == CHAIN_ID and funding["roles"]["deployer"].lower() == identity["deployerAddress"].lower(), "funded deployer identity changed")
    require(identity["funding"]["finalizedBlock"] == funding["finalizedBlock"] and identity["funding"]["finalizedHash"].lower() == funding["finalizedHash"].lower() and identity["funding"]["markerSha256"] == digest(marker_path), "immutable funding marker does not match reservation")
    require(roles["testOnly"] is True and roles["roles"]["deployer"].lower() == identity["deployerAddress"].lower() and roles["roles"]["root"].lower() == funding["roles"]["root"].lower(), "fresh Hoodi roles mismatch")
    nonce = identity["nonce"]
    require(identity["messageQueueCreateNonce"] == spec["messageQueueCreateNonce"] == 12 and nonce["value"] == nonce["latest"] == nonce["pending"] == spec["reservedNonce"] == 0 and nonce["blockTag"] == "latest/pending" and identity["deployerTransactionsBroadcastByThisSlice"] is False, "deployer nonce-zero reservation changed")
    deployer = hex_bytes(identity["deployerAddress"], 20, "deployer")
    queue = to_checksum_address(keccak(rlp.encode([deployer, 12]))[-20:])
    source = hex_bytes(identity["sourceDomain"], 32, "source domain")
    preimage = b"vara/gear-eth-bridge-domain/v2" + source + CHAIN_ID.to_bytes(32, "big") + bytes.fromhex(queue[2:])
    bridge = "0x" + keccak(preimage).hex()
    require(identity["destinationQueue"].lower() == spec["destinationQueue"].lower() == queue.lower(), "CREATE nonce-12 queue identity mismatch")
    require(identity["bridgeDomainPreimage"].lower() == "0x" + preimage.hex() and identity["bridgeDomain"].lower() == spec["bridgeDomain"].lower() == bridge, "v2 bridge domain preimage mismatch")
    require(spec["rawSpecPath"] == str(RUN / "source-chain/chain.raw.json") and digest(Path(spec["rawSpecPath"])) == spec["rawSpecSha256"], "source raw spec changed")
    require(launch["phase"] == "ready" and launch["identity"]["bridgeDomain"].lower() == bridge and launch["readiness"]["genesisHash"].lower() == stack["sourceGenesis"].lower(), "native source is not ready for this domain/genesis")
    require(launch["identity"]["rawSpecSha256"].removeprefix("0x") == spec["rawSpecSha256"] and launch["readiness"]["validatorCount"] == 2, "raw spec or validator count changed")
    require(launch["identity"]["mmrStartBlock"] == anchor["mmrStartBlock"] and anchor["beefyActivationBlock"] == launch["identity"]["beefyActivationBlock"], "source MMR/BEEFY activation changed")
    require(stack["lane"] == "beefy-token-hoodi" and stack["sourceGenesis"].lower() == anchor["sourceGenesis"].lower() == launch["readiness"]["genesisHash"].lower(), "token stack or signed anchor has another source genesis")
    require(anchor["bridgeDomain"].lower() == bridge, "signed anchor has another bridge domain")
    require(uint(anchor["block"], 64, "anchor block") > anchor["mmrStartBlock"] and anchor["freshnessSourceBlock"] == anchor["block"] - 1, "invalid signed anchor height")
    hex_bytes(anchor["blockHash"], 32, "anchor block hash")
    hex_bytes(anchor["mmrRoot"], 32, "anchor MMR root")
    require(len(bytes.fromhex(anchor["signedCommitmentScale"][2:])) > 64, "signed BEEFY commitment absent")
    uint(anchor["sourceTimestampMs"], 64, "source timestamp")
    alice, bob = launch["aliceRpc"], launch["bobRpc"]
    require(alice != bob, "independent Gear witness endpoints required")
    for endpoint in (alice, bob):
        require(gear_rpc(endpoint, "chain_getBlockHash", 0).lower() == anchor["sourceGenesis"].lower(), "Gear source genesis changed")
        require(gear_rpc(endpoint, "chain_getBlockHash", anchor["block"]).lower() == anchor["blockHash"].lower(), "signed anchor no longer canonical on both Gear validators")
        head = gear_rpc(endpoint, "chain_getFinalizedHead")
        require(int(gear_rpc(endpoint, "chain_getHeader", head)["number"], 16) >= anchor["block"], "Gear validator has not finalized signed anchor")
    for name in ("current", "next"):
        authorities = anchor[name]
        uint(authorities["id"], 128, f"{name} authority id")
        require(len(authorities["keys"]) == 2, f"{name} two-authority set missing")
        for authority in authorities["keys"]:
            hex_bytes("0x" + bytes(authority).hex() if isinstance(authority, list) else authority, 33, f"{name} authority key")
        root = authorities["root"]
        hex_bytes("0x" + bytes(root).hex() if isinstance(root, list) else root, 32, f"{name} authority root")
    require(anchor["next"]["id"] >= anchor["current"]["id"], "anchor authority handover regressed")
    programs = stack["programs"]
    names = ("vftManager", "bridgingPayment", "circleVft", "tetherVft", "etherVft", "bitcoinVft", "gearOriginVft", "nativeVft", "historicalProxy", "ethEventsElectra")
    program_ids = []
    for name in names:
        program = programs[name]
        program_ids.append(hex_bytes(program["id"], 32, name))
        require(program["status"] == "active" and bytes.fromhex(program["salt"][2:]) == f"beefy-token-hoodi-{stack['sourceGenesis']}-{name}".encode(), f"{name} not a fresh active program")
    require(len(set(program_ids)) == len(program_ids), "token stack reuses a program identity")
    require(hex_bytes(stack["checkpoint"], 32, "checkpoint") not in program_ids and uint(stack["checkpointSlot"], 64, "checkpoint slot") > 0, "checkpoint identity invalid")
    hex_bytes(stack["checkpointHash"], 32, "checkpoint hash")
    require(len(set(hex_bytes(gear_roles[name]["publicKey"], 32, name) for name in ("checkpoint", "inbound", "campaign", "governance", "rotation", "paid"))) == 6, "fresh Gear role identities missing")
    # Runtime PalletId py/gethb, SCALE-encoded subaccounts (not the setup/governance signer).
    builtins = {label: "0x" + (b"modlpy/gethb" + bytes([len(label) << 2]) + label).ljust(32, b"\0").hex() for label in (b"bridge_admin", b"bridge_pauser")}
    gear_admin, gear_pauser = builtins[b"bridge_admin"], builtins[b"bridge_pauser"]
    require(gear_admin != gear_pauser and hex_bytes(gear_admin, 32, "bridge admin") not in program_ids and hex_bytes(gear_pauser, 32, "bridge pauser") not in program_ids, "builtin governance identity invalid")
    now_ms = int(rpc("eth_getBlockByNumber", ["latest", False])["timestamp"], 16) * 1000
    stamp = anchor["sourceTimestampMs"]
    require(now_ms - 24 * 60 * 60 * 1000 < stamp <= now_ms + 120_000, "signed source timestamp outside BeefyClient live window")
    return {"identity": identity, "funding": funding, "stack": stack, "anchor": anchor, "roles": roles["roles"], "gearAdmin": gear_admin, "gearPauser": gear_pauser,
            "queue": queue, "bridge": bridge, "source": "0x" + source.hex(), "genesis": stack["sourceGenesis"],
            "inputSha256": {"networkGate": digest(network_gate_path), "sourceIdentity": digest(identity_path), "sourceLaunchState": digest(RUN / "source-chain/launch-state.json"), "rawSpec": spec["rawSpecSha256"], "tokenStack": digest(stack_path), "anchor": digest(anchor_path)}}


def funding_and_chain(inputs, deployer):
    require(int(rpc("eth_chainId", []), 16) == CHAIN_ID and rpc("eth_getBlockByNumber", ["0x0", False])["hash"].lower() == HOODI_GENESIS, "wrong Hoodi chain/genesis")
    finalized = rpc("eth_getBlockByNumber", ["finalized", False])
    finalized_height = int(finalized["number"], 16)
    marker = inputs["funding"]
    height, recorded_hash = marker["finalizedBlock"], marker["finalizedHash"]
    require(height <= finalized_height and rpc("eth_getBlockByNumber", [hex(height), False])["hash"].lower() == recorded_hash.lower(), "recorded funding finality not canonical")
    require(rpc("eth_getCode", [inputs["queue"], "latest"]) == "0x", "CREATE-12 queue already contains code: reconcile deployment")
    require(int(rpc("eth_getBalance", [deployer, "latest"]), 16) > 0, "deployer unfunded")
    return {"finalizedHeight": finalized_height}


def nonce_zero(deployer, reservation_rpc):
    for endpoint in {RPC, reservation_rpc}:
        latest = int(rpc("eth_getTransactionCount", [deployer, "latest"], endpoint), 16)
        pending = int(rpc("eth_getTransactionCount", [deployer, "pending"], endpoint), 16)
        require(latest == pending == 0, "reserved deployer nonce changed or pending transaction exists; reconcile, never rerun")


def run_forge(command, environment, log_path, secrets, lock_fd, *, exclusive):
    flags = os.O_WRONLY | os.O_CREAT | (os.O_EXCL if exclusive else os.O_APPEND)
    log_fd = os.open(log_path, flags, 0o600)
    with os.fdopen(log_fd, "a") as log:
        with subprocess.Popen(command, cwd=PROJECT, env=environment, stdout=subprocess.PIPE,
                              stderr=subprocess.STDOUT, text=True, errors="replace", pass_fds=(lock_fd,)) as process:
            for line in process.stdout:
                for secret in secrets:
                    if secret:
                        line = line.replace(secret, "[REDACTED]").replace(secret.upper(), "[REDACTED]")
                log.write(line)
            result = process.wait()
        log.flush()
        os.fsync(log.fileno())
    require(result == 0, "Forge failed; preserve the private log and reconcile any original broadcast before retrying")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--check", action="store_true", help="validate readiness without writing intent or broadcasting")
    mode.add_argument("--dry-run", action="store_true", help="simulate exact sealed inputs without broadcasting")
    options = parser.parse_args()
    os.umask(0o077)
    with (PROJECT / "deployment.lock").open("a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        intent_path = PROJECT / "deployment-intent.json"
        log_path = PROJECT / "deployment.log"
        require(not intent_path.exists(), "existing intent: reconcile original attempt; rerun prohibited")
        dry_run = PROJECT / "broadcast/BeefyTokens.s.sol/560048/dry-run/run-latest.json"
        for journal_path in (PROJECT / "broadcast").rglob("*.json"):
            journal = load(journal_path)
            require(journal_path.parent.name == "dry-run" and journal["chain"] == CHAIN_ID and not journal["receipts"] and not journal["pending"] and all(item["hash"] is None for item in journal["transactions"]), "existing broadcast evidence: rerun prohibited")
        built = compiled_artifact()
        inputs = source_inputs()
        private_key, deployer = key(RUN / "hoodi/keys/deployer.key")
        require(deployer.lower() == inputs["identity"]["deployerAddress"].lower(), "reserved deployer private key changed")
        chain = funding_and_chain(inputs, deployer)
        nonce_zero(deployer, inputs["identity"]["nonce"]["rpc"])
        environment = {
            "FOUNDRY_EXTRA_OUTPUT_FILES": "[]",  # Use sealed IR sidecars; regeneration invalidates the portable cache.
            "EXPECTED_DEPLOYER_NONCE": "0", "GEAR_VFT_MANAGER": inputs["stack"]["programs"]["vftManager"]["id"],
            "GEAR_GOVERNANCE_ADMIN": inputs["gearAdmin"], "GEAR_GOVERNANCE_PAUSER": inputs["gearPauser"],
            "BEEFY_SOURCE_DOMAIN": inputs["source"], "BEEFY_BRIDGE_DOMAIN": inputs["bridge"],
            "BEEFY_MMR_START_BLOCK": str(inputs["anchor"]["mmrStartBlock"]),
            "BEEFY_INITIAL_BLOCK": str(inputs["anchor"]["block"]),
            "BEEFY_INITIAL_SOURCE_TIMESTAMP_MS": str(inputs["anchor"]["sourceTimestampMs"]),
            "BRIDGING_PAYMENT_FEE": str(FEE_WEI), "EMERGENCY_STOP_ADMIN": deployer,
            "EMERGENCY_STOP_OBSERVER": inputs["roles"]["root"],
        }
        for name in ("current", "next"):
            set_ = inputs["anchor"][name]
            prefix = "BEEFY_" + name.upper()
            root = set_["root"]
            environment.update({prefix + "_ID": str(set_["id"]), prefix + "_LENGTH": str(len(set_["keys"])), prefix + "_ROOT": "0x" + bytes(root).hex() if isinstance(root, list) else root})
        clean_env = {name: os.environ[name] for name in ("PATH", "HOME", "ETHERSCAN_API_KEY") if name in os.environ}
        clean_env.update(environment)
        clean_env["PRIVATE_KEY"] = private_key
        secrets = (private_key, private_key[2:], str(int(private_key, 16)), clean_env.get("ETHERSCAN_API_KEY", ""))
        command = ["rtk", "forge", "script", "--root", str(PROJECT), "script/BeefyTokens.s.sol:BeefyTokens", "--rpc-url", RPC, "-vvvv"]
        simulation_identity = {"environment": environment, "compiled": built, "sourceFilesSha256": inputs["inputSha256"], "deployer": deployer, "rpc": RPC}
        simulation_path = PROJECT / "simulation.json"
        require(compiled_artifact() == built, "Deployment artifacts or Foundry inputs changed during readiness checks")
        if options.dry_run:
            run_forge(command, clean_env, PROJECT / "simulation.log", secrets, lock.fileno(), exclusive=False)
            require(compiled_artifact() == built, "Simulation changed deployment artifacts or Foundry inputs; deployment held")
            require(dry_run.is_file(), "Forge did not produce a dry-run journal")
            journal = load(dry_run)
            require(journal["chain"] == CHAIN_ID and not journal["receipts"] and not journal["pending"]
                    and journal["transactions"] and all(item["hash"] is None for item in journal["transactions"]), "Unexpected simulation transaction evidence")
            from run_context import save
            save(simulation_path, {**simulation_identity, "dryRunSha256": digest(dry_run)})
            print("Exact sealed script simulated; no transaction broadcast. Validate with --check before deployment.")
            return
        require(dry_run.is_file() and simulation_path.is_file(), "Run --dry-run with this sealed script and run descriptor first")
        require(load(simulation_path) == {**simulation_identity, "dryRunSha256": digest(dry_run)}, "Simulation inputs or artifact changed; deployment held")
        require(clean_env.get("ETHERSCAN_API_KEY"), "ETHERSCAN_API_KEY required for --verify; do not put it in arguments")
        if options.check:
            print("Fresh BeefyTokens preflight passed; no deployment intent written and no transaction submitted.")
            return
        command += ["--broadcast", "--verify"]
        intent = {"phase": "intent-recorded", "testOnly": True, "runId": RUN.name, "chainId": CHAIN_ID,
                  "deployer": deployer, "reservedNonce": 0, "queueCreateNonce": 12, "predictedQueue": inputs["queue"],
                  "sourceGenesis": inputs["genesis"], "anchorBlock": inputs["anchor"]["block"], "anchorBlockHash": inputs["anchor"]["blockHash"],
                  "environment": environment, "sourceFilesSha256": inputs["inputSha256"], "compiled": built,
                  "chain": chain, "command": command, "privateKeySource": "hoodi/keys/deployer.key (mode 0600, environment only)",
                  "rpc": RPC, "log": str(log_path), "dryRunSha256": digest(dry_run)}
        fd = os.open(intent_path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as output:
            json.dump(intent, output, sort_keys=True, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        directory = os.open(PROJECT, os.O_RDONLY)
        try:
            os.fsync(directory)
        finally:
            os.close(directory)
        nonce_zero(deployer, inputs["identity"]["nonce"]["rpc"])
        require(rpc("eth_getCode", [inputs["queue"], "latest"]) == "0x", "queue changed after intent: reconcile, never rerun")
        require(compiled_artifact() == built, "Deployment artifacts or Foundry inputs changed after intent; reconcile, never rerun")
        run_forge(command, clean_env, log_path, secrets, lock.fileno(), exclusive=True)
        print("Forge completed. Reconcile original broadcast receipts and verify deployed identities; deployment intent cannot be reused.")


if __name__ == "__main__":
    try:
        main()
    except (OSError, KeyError, ValueError, RuntimeError, TypeError, IndexError, OverflowError) as error:
        # Never print private-key values, RPC transaction calldata, or forge environment.
        raise SystemExit(f"Deployment held: {error}") from None
