#!/usr/bin/env python3
"""Queue and index the original nonce-0 nonasset bootstrap; campaign readiness authenticates it."""
import argparse
import fcntl
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import uuid

from eth_account import Account
from eth_utils import keccak
from substrateinterface import Keypair, SubstrateInterface

def bootstrap_assets_are_unbridged(assets, source_inventory):
    source_assets = {} if source_inventory is None else source_inventory["assets"]
    if source_inventory is not None and (source_inventory["status"] != "ready" or set(source_assets) != {"GOT", "WTVARA"}):
        return False
    if set(assets) != {"USDC", "USDT", "WETH", "WBTC"} | set(source_assets):
        return False
    for symbol, asset in assets.items():
        if symbol in source_assets:
            amount = int(source_assets[symbol]["amountRaw"])
            if amount <= 0 or int(asset["gearUser"]) != amount or int(asset["vftSupply"]) != amount:
                return False
            empty = ("evmManagerEscrow", "gearManagerEscrow", "evmUser", "erc20Supply")
        else:
            empty = ("evmManagerEscrow", "gearUser", "vftSupply")
        if any(int(asset[name]) != 0 for name in empty):
            return False
    return True


def main():
    from run_context import RUN, OPS, CONFIG, artifact, digest, load, require, rpc, save
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("queue", "record"))
    stage = parser.parse_args().stage
    lock = (RUN / "queue-bootstrap.lock").open("a")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    deployment = load(RUN / "deployment.json")
    stack = load(RUN / "token-stack/token-stack.json")
    identity = load(RUN / "source-chain/identity.json")
    roles = load(RUN / "hoodi/gear-addresses.json")["roles"]
    addresses = load(RUN / "hoodi/addresses.json")["roles"]
    require(stack["configuration"]["status"] == "ready", "Token configuration must be complete")
    require(load(RUN / "source-chain/setup/bridge-ready.json")["phase"] == "ready", "Original source initialization must be complete")
    spec = importlib.util.spec_from_file_location("source_setup", OPS / "source-chain/initialize-bridge.py")
    setup = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(setup)
    api = SubstrateInterface(url=CONFIG["source"]["aliceRpc"], ss58_format=42)
    genesis = api.get_block_hash(0)
    require(genesis == deployment["anchor"]["sourceGenesis"] == stack["sourceGenesis"], "Source genesis mismatch")
    require(deployment["anchor"]["bridgeDomain"] == identity["bridgeDomain"], "Bridge domain mismatch")
    environment = load(RUN / "forge-final/deployment-intent.json")["environment"]
    source = environment["GEAR_GOVERNANCE_PAUSER"]
    require(api.get_constant("GearEthBridge", "BridgePauser").data.to_hex().lower() == source.lower(), "Deployed pauser differs from source runtime")
    destination = deployment["ethereumConfiguration"]["governancePauser"]
    payload = "0x00" + source.removeprefix("0x")
    message = {"nonce": "0", "source": source, "destination": destination, "payload": payload}
    before_path = RUN / "follower/bootstrap-before.json"
    message_path = setup.SETUP / "queue-bootstrap.message.json"
    intent_path = setup.SETUP / "queue-bootstrap.intent.json"
    receipt_path = setup.SETUP / "queue-bootstrap.finalized.json"
    marker = RUN / "follower/queue-bootstrap.json"

    def snapshot(path):
        command = [str(artifact("beefy-relay")), "tokens-snapshot", "--source-rpc", CONFIG["source"]["aliceRpc"],
                   "--witness-rpc", CONFIG["source"]["bobRpc"], "--ethereum-rpc", CONFIG["network"]["executionWss"],
                   "--beacon-rpc", CONFIG["network"]["beaconHttp"], "--deployment-manifest", str(RUN / "deployment.json"),
                   "--token-stack", str(RUN / "token-stack/token-stack.json"), "--gear-user", roles["campaign"]["publicKey"],
                   "--evm-user", addresses["campaign"], "--output", str(path)]
        subprocess.run(command, check=True, timeout=120, pass_fds=(lock.fileno(),))
        return load(path)

    if stage == "queue":
        require(not marker.exists(), "Bootstrap already indexed; do not queue another message")
        if not before_path.exists():
            require(not intent_path.exists(), "Original before-snapshot is missing; HOLD")
            snapshot(before_path)
        before = load(before_path)
        inventory = stack["sourceInventory"] if CONFIG.get("runtimeProfile") is not None else None
        require(bootstrap_assets_are_unbridged(before["assets"], inventory), "Incomplete asset snapshot or bridge liabilities precede bootstrap")
        require(api.get_block_hash(before["gearHeight"]) == before["gearHash"], "Original before-snapshot became noncanonical")
        nonce = api.query("GearEthBridge", "MessageNonce", block_hash=before["gearHash"]).value
        require(nonce is None or int(nonce) == 0, "Source messages precede bootstrap")
        setup.write_once(message_path, {"sourceGenesis": genesis, "bridgeDomain": identity["bridgeDomain"],
                         "message": message, "beforeSnapshotSha256": digest(before_path)})
        if not intent_path.exists():
            _, head = setup.finalized(api)
            nonce = api.query("GearEthBridge", "MessageNonce", block_hash=head).value
            require(nonce is None or int(nonce) == 0, "Nonce 0 is no longer available; HOLD")
        inner = api.compose_call("GearEthBridge", "send_eth_message", {"destination": destination, "payload": payload})
        call = api.compose_call("Sudo", "sudo_as", {"who": source, "call": inner.value})
        receipt = setup.submit_once(api, "queue-bootstrap", call, Keypair.create_from_uri("//Alice"), genesis,
                                    ["GearEthBridge", "MessageQueued"])
        require("Sudo.SudoAsDone" in receipt["events"], "Bootstrap lacked its sudo_as outcome")
        print("Original nonasset bootstrap queued. Wait for independent follower and paid-worker finality, then run record.")
        return

    require(not marker.exists(), "Bootstrap evidence already indexed; readiness must revalidate it, not replace it")
    original = load(message_path)
    require(original == {"sourceGenesis": genesis, "bridgeDomain": identity["bridgeDomain"],
                         "message": message, "beforeSnapshotSha256": digest(before_path)}, "Original bootstrap identity changed")
    receipt = load(receipt_path)
    intent = load(intent_path)
    require(setup.scan(api, intent, receipt["block"]) == receipt, "Original source handoff is not canonical")
    status_path = RUN / "outbound/journal/transaction_status.json"
    delivery_status = load(status_path)
    require(delivery_status == {"version": 1, "active": {}, "failed": []}, "Outbound delivery liabilities are not empty; HOLD")
    completed = []
    for path in (RUN / "outbound/journal").iterdir():
        try:
            identifier = str(uuid.UUID(path.name))
        except ValueError:
            continue
        delivery = load(path)
        observed = delivery.get("message", {}).get("message", {})
        if (observed.get("nonce_be") == [0] * 32 and observed.get("source") == list(bytes.fromhex(source[2:]))
                and observed.get("destination") == list(bytes.fromhex(destination[2:]))
                and observed.get("payload") == list(bytes.fromhex(payload[2:]))):
            require(delivery["journal_version"] == 3 and delivery["uuid"] == identifier, "Ambiguous worker journal identity")
            require(delivery["message"]["block"] == receipt["block"] and delivery["message"]["block_hash"] == receipt["blockHash"],
                    "Worker source inclusion differs from the original extrinsic")
            if delivery["status"] == "Completed":
                completed.append(delivery)
    require(len(completed) == 1, "The original bootstrap delivery is not uniquely Completed; HOLD")
    delivery = completed[0]
    submission = delivery["ethereum_submission"]
    process_hash = delivery["ethereum_tx_hash"]
    require(process_hash == submission["hash"] and delivery["ethereum_tx_attempts"] == [process_hash], "Delivery is not the original signed transaction")
    raw = bytes(submission["raw_transaction"])
    require("0x" + keccak(raw).hex() == process_hash
            and Account.recover_transaction(raw).lower() == submission["sender"].lower() == addresses["paid"].lower()
            and submission["chain_id"] == CONFIG["network"]["chainId"]
            and submission["contract"].lower() == deployment["ethereum"]["queue"].lower(), "Delivery signature/lane mismatch")
    root = "0x" + bytes(delivery["message_hash"]).hex()
    require(len(root) == 66 and root != "0x" + "00" * 32, "Invalid first queue root")
    key = str(receipt["block"]) + "-" + root[2:]
    state = load(RUN / "follower/state.json")
    entry = state["roots"][key]
    require(entry["kind"] == "merkleRoot" and entry["block"] == receipt["block"]
            and entry["blockHash"] == receipt["blockHash"] and entry["queueRoot"].lower() == root, "First root differs from original source message")
    publication_path = "root-publications/" + key + ".json"
    publication = load(RUN / "follower" / publication_path)
    require(publication["schemaVersion"] == 3 and publication["sourceBlock"] == receipt["block"]
            and publication["root"].lower() == root
            and publication["acceptedAnchorClient"].lower() == deployment["ethereum"]["client"].lower(), "Publication identity mismatch")
    anchors = [entry for entry in state["commitments"] if entry.get("txHash") == publication["acceptedAnchorTx"]]
    require(len(anchors) == 1 and anchors[0]["block"] == publication["proof"]["anchorBlock"], "Original accepted anchor is ambiguous")
    inclusions = {}
    finalized = rpc("eth_getBlockByNumber", ["finalized", False])
    for name, tx_hash in (("anchor", publication["acceptedAnchorTx"]), ("root", publication["txHash"]), ("process", process_hash)):
        result = rpc("eth_getTransactionReceipt", [tx_hash])
        require(result is not None and result["status"] == "0x1" and result["transactionHash"].lower() == tx_hash.lower(), "Original " + name + " receipt unavailable")
        require(int(result["blockNumber"], 16) <= int(finalized["number"], 16)
                and rpc("eth_getBlockByNumber", [result["blockNumber"], False])["hash"] == result["blockHash"], "Original " + name + " receipt is not canonically finalized")
        inclusions[name] = int(result["blockNumber"], 16)
    before = load(before_path)
    after = snapshot(RUN / "follower" / ("bootstrap-after-" + str(uuid.uuid4()) + ".json"))
    require(before["gearHeight"] < receipt["block"] <= after["gearHeight"]
            and before["evmHeight"] < inclusions["root"]
            and after["evmHeight"] >= max(inclusions.values()), "Snapshots do not bracket finalized bootstrap; HOLD")
    require(before["assets"] == after["assets"], "Bootstrap changed asset accounting; HOLD")
    require(load(status_path) == delivery_status, "Outbound delivery view changed during bootstrap observation; HOLD")
    save(marker, {"schemaVersion": 1, "sourceGenesis": genesis, "bridgeDomain": identity["bridgeDomain"],
         "sourceBlock": receipt["block"], "sourceHash": receipt["blockHash"], "message": message,
         "sourceSubmission": {key: receipt[key] for key in ("extrinsicHash", "block", "blockHash", "extrinsicIndex")},
         "acceptedAnchorTx": publication["acceptedAnchorTx"], "rootPublication": {"path": publication_path, "txHash": publication["txHash"]},
         "processTxHash": process_hash, "outboundTransactionUuid": delivery["uuid"], "beforeSnapshot": before, "afterSnapshot": after})
    print("Bootstrap evidence indexed. Campaign readiness must independently authenticate every original receipt, proof, and historical balance.")


if __name__ == "__main__":
    main()
