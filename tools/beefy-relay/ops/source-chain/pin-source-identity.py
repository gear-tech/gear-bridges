#!/usr/bin/env python3
"""Pin the fresh source domain only after independent Hoodi funding finality."""
import hashlib
import json
import os
import secrets
import urllib.request
from datetime import datetime, timezone
from pathlib import Path
from Crypto.Hash import keccak

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from run_context import RUN, RPC, CONFIG
CHAIN_ID = 560048
GENESIS = "0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b"
PREFIX = b"vara/gear-eth-bridge-domain/v2"
MARKER = RUN / "hoodi" / "funding-complete.json"
ADDRESS_FILE = RUN / "hoodi" / "addresses.json"
OUTPUT = RUN / "source-chain" / "identity.json"


def call(method, params=()):
    req = urllib.request.Request(RPC, json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": list(params)}).encode(), {"User-Agent": "Mozilla/5.0", "Content-Type": "application/json"})
    with urllib.request.urlopen(req, timeout=30) as response:
        data = json.load(response)
    if "error" in data or "result" not in data:
        raise RuntimeError(f"Hoodi RPC {method} failed")
    return data["result"]


def digest(data):
    h = keccak.new(digest_bits=256)
    h.update(data)
    return h.digest()


def checksum(address):
    lower = address.hex()
    mask = digest(lower.encode()).hex()
    return "0x" + "".join(c.upper() if int(mask[i], 16) >= 8 else c for i, c in enumerate(lower))


def require(condition, description):
    if not condition:
        raise RuntimeError(description)


def main():
    require(not OUTPUT.exists(), "source identity is already reserved; refusing a replacement domain")
    marker = json.loads(MARKER.read_text())
    addresses = json.loads(ADDRESS_FILE.read_text())["roles"]
    require(marker.get("phase") == "finalized" and marker.get("chainId") == CHAIN_ID, "funding marker lacks finalized Hoodi identity")
    roles = marker.get("roles", {})
    require(set(("deployer", "follower", "root", "paid", "campaign")) <= set(roles) <= set(addresses), "funding roles do not match fresh run")
    require(all(isinstance(addresses[role], str) and roles[role].lower() == addresses[role].lower() for role in roles), "funding marker roles mismatch fresh run")
    require(len({value.lower() for value in roles.values()}) == len(roles), "duplicate funded EVM roles")
    require(int(call("eth_chainId"), 16) == CHAIN_ID, "not Hoodi")
    require(call("eth_getBlockByNumber", ["0x0", False])["hash"].lower() == GENESIS, "Hoodi genesis mismatch")
    height = marker["finalizedBlock"]
    block = call("eth_getBlockByNumber", [hex(height), False])
    finalized = call("eth_getBlockByNumber", ["finalized", False])
    require(block and block["hash"].lower() == marker["finalizedHash"].lower(), "funding marker block is not canonical")
    require(finalized and int(finalized["number"], 16) >= height, "funding marker is not finalized")
    transaction_hashes = marker.get("transactionHashes", [])
    require(len(transaction_hashes) >= 5 and len(set(transaction_hashes)) == len(transaction_hashes), "funding receipts missing or repeated")
    funded = set()
    for tx_hash in transaction_hashes:
        receipt = call("eth_getTransactionReceipt", [tx_hash])
        require(receipt and receipt["status"] == "0x1", "funding transaction is not successful")
        number = int(receipt["blockNumber"], 16)
        canonical = call("eth_getBlockByNumber", [hex(number), False])
        require(number <= height and canonical and canonical["hash"].lower() == receipt["blockHash"].lower(), "funding receipt is not canonical/finalized")
        tx = call("eth_getTransactionByHash", [tx_hash])
        require(tx and tx["from"].lower() == marker["source"].lower() and int(tx["value"], 16) > 0, "funding transaction source/value mismatch")
        funded.add(tx["to"].lower())
    require({roles[role].lower() for role in roles} <= funded, "funded destination set differs from fresh EVM roles")
    for role in roles:
        require(int(call("eth_getBalance", [roles[role], hex(height)]), 16) > 0, f"{role} has no finalized funding balance")
    deployer = addresses["deployer"]
    latest = int(call("eth_getTransactionCount", [deployer, "latest"]), 16)
    pending = int(call("eth_getTransactionCount", [deployer, "pending"]), 16)
    require(latest == pending == 0, "fresh deployer nonce zero is no longer available")
    sender = bytes.fromhex(deployer[2:])
    queue = checksum(digest(bytes([0xd6, 0x94]) + sender + bytes([12]))[-20:])
    source_domain = secrets.token_bytes(32)
    require(source_domain != bytes(32), "zero source domain")
    preimage = PREFIX + source_domain + CHAIN_ID.to_bytes(32, "big") + bytes.fromhex(queue[2:])
    identity = {
        "schemaVersion": 1, "runId": RUN.name, "purpose": "fresh native local Gear source bound to Hoodi; test only",
        "destinationChainId": CHAIN_ID, "deployerAddress": deployer,
        "sourceDomain": "0x" + source_domain.hex(), "bridgeDomainPreimage": "0x" + preimage.hex(),
        "bridgeDomain": "0x" + digest(preimage).hex(),
        "nonce": {"value": 0, "latest": latest, "pending": pending, "blockTag": "latest/pending", "rpc": RPC, "reservation": "reserved by this private run; no deployer transaction broadcast"},
        "messageQueueCreateNonce": 12, "destinationQueue": queue,
        "funding": {"markerPath": str(MARKER), "phase": "finalized", "finalizedBlock": height, "finalizedHash": marker["finalizedHash"], "markerSha256": hashlib.sha256(MARKER.read_bytes()).hexdigest()},
        "reservedAt": datetime.now(timezone.utc).isoformat(), "deployerTransactionsBroadcastByThisSlice": False,
    }
    if CONFIG.get("runtimeProfile") is not None:
        identity["runtimeProfile"] = CONFIG["runtimeProfile"]
    os.umask(0o077)
    fd = os.open(OUTPUT, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as target:
        target.write(json.dumps(identity, indent=2) + chr(10))
        target.flush()
        os.fsync(target.fileno())
    print(json.dumps({k: identity[k] for k in ("runId", "sourceDomain", "bridgeDomain", "destinationQueue", "deployerAddress", "nonce", "funding")}, sort_keys=True))


if __name__ == "__main__":
    main()
