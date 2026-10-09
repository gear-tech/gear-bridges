#!/usr/bin/env python3
"""Prepare one fresh local Hoodi run from a sealed bundle; no funding or deployment."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import re
import shutil
import socket
import sys
import uuid

from substrateinterface import Keypair

def relocated_compiler_cache(cache, source_root, project):
    """Relocate only Foundry's absolute remapping targets, preserving every other field."""
    project = project.resolve()
    result = {**cache, "remappings": []}
    for mapping in cache["remappings"]:
        alias, separator, target = mapping.partition("=")
        if not separator or not Path(target).is_absolute() or ".." in Path(target).parts:
            raise ValueError("Invalid sealed compiler remapping")
        relative = Path(target).relative_to(source_root)
        result["remappings"].append(alias + "=" + str(project / relative) + ("/" if target.endswith("/") else ""))
    return result



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runs-dir", type=Path, required=True)
    parser.add_argument("--funding-wallet", type=Path, required=True)
    parser.add_argument("--funding-address", required=True)
    parser.add_argument("--execution-http", default="https://rpc.sentio.xyz/hoodi")
    parser.add_argument("--execution-wss", default="wss://rpc.sentio.xyz/hoodi")
    parser.add_argument("--beacon-http", default="https://ethereum-hoodi-beacon-api.publicnode.com")
    parser.add_argument("--trusted-bootstrap-root", required=True,
                        help="Independently approved recent Ethereum weak-subjectivity checkpoint root")
    parser.add_argument("--trusted-genesis-validators-root", required=True,
                        help="Independently approved Hoodi Beacon genesis validators root")
    parser.add_argument("--runtime-profile", choices=("legacy-hoodi", "normal-runtime-hoodi", "fast-runtime-hoodi"), required=True)
    parser.add_argument("--source-rpc-port", type=int, default=9962)
    parser.add_argument("--source-p2p-port", type=int, default=30362)
    parser.add_argument("--source-metrics-port", type=int, default=9629)
    parser.add_argument("--service-port", type=int, default=19151)
    args = parser.parse_args()
    if not re.fullmatch(r"0x[0-9a-fA-F]{64}", args.trusted_bootstrap_root) or int(args.trusted_bootstrap_root, 16) == 0:
        parser.error("Trusted bootstrap root must be a nonzero 32-byte hex value")
    if args.trusted_genesis_validators_root.lower() != "0x212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f":
        parser.error("Trusted genesis validators root does not identify Hoodi")
    if not __debug__:
        raise SystemExit("Non-optimized Python required")
    bundle = Path(__file__).resolve().parent.parent
    with (bundle / "bundle.json").open("rb") as stream:
        bundle_hash = hashlib.file_digest(stream, "sha256").hexdigest()
    manifest = json.loads((bundle / "bundle.json").read_text())
    profile = manifest.get("runtimeProfile")
    if args.runtime_profile != "legacy-hoodi":
        import runpy
        if not isinstance(profile, dict) or profile.get("name") != args.runtime_profile:
            parser.error("Requested runtime profile differs from the sealed artifact selection")
        runpy.run_path(str(Path(__file__).with_name("setup-services.py")))["runtime_profile"](profile, manifest["files"]["bin/gear"])
        if hashlib.sha256((bundle / "runtime-approval.json").read_bytes()).hexdigest() != profile["approvalSha256"].removeprefix("0x"):
            parser.error("Independent runtime selection changed; HOLD")
    elif profile is not None:
        parser.error("Profiled-runtime bundle cannot be admitted as a legacy campaign")
    ports = [args.source_rpc_port, args.source_rpc_port + 1, args.source_p2p_port,
             args.source_p2p_port + 1, args.source_metrics_port, args.source_metrics_port + 1,
             *range(args.service_port, args.service_port + 4)]
    if len(set(ports)) != len(ports) or any(not 1024 <= port <= 65535 for port in ports):
        raise SystemExit("Ports must be distinct non-privileged TCP ports")
    # ponytail: probes do not reserve ports; supervisors fail closed if another process claims one.
    sockets = []
    try:
        for port in ports:
            sock = socket.socket()
            sockets.append(sock)
            sock.bind(("127.0.0.1", port))
    finally:
        for sock in sockets:
            sock.close()
    os.umask(0o077)
    run_id = str(uuid.uuid4())
    run = args.runs_dir.resolve(strict=True) / run_id
    run.mkdir(mode=0o700)
    config = {"schemaVersion": 1, "testOnly": True, "runId": run_id,
              "bundle": {"path": str(bundle), "sha256": bundle_hash},
              "network": {"chainId": 560048,
                          "genesisHash": "0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b",
                          "executionHttp": args.execution_http, "executionWss": args.execution_wss,
                          "beaconHttp": args.beacon_http,
                          "trustedBootstrapRoot": args.trusted_bootstrap_root.lower(),
                          "beaconGenesisValidatorsRoot": args.trusted_genesis_validators_root.lower()},
              "source": {"aliceRpc": "ws://127.0.0.1:" + str(args.source_rpc_port),
                         "bobRpc": "ws://127.0.0.1:" + str(args.source_rpc_port + 1),
                         "p2pPorts": {"alice": args.source_p2p_port, "bob": args.source_p2p_port + 1},
                         "metricsPorts": {"alice": args.source_metrics_port, "bob": args.source_metrics_port + 1}},
              "services": {"ports": dict(zip(("checkpoint", "inbound", "outbound", "web"), range(args.service_port, args.service_port + 4)))},
              "funding": {"wallet": str(args.funding_wallet.resolve(strict=True)), "address": args.funding_address}}
    if profile is not None:
        config["runtimeProfile"] = profile
    with (run / "run.json").open("x") as stream:
        json.dump(config, stream, sort_keys=True, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.environ["BEEFY_RUN"] = str(run)
    from run_context import MANIFEST, NETWORK, checked_file, require, request, rpc, save, wallet
    for relative in MANIFEST["files"]:
        checked_file(relative)
    require(wallet(config["funding"]["wallet"])["address"].lower() == args.funding_address.lower(), "Funding identity mismatch")
    require(int(rpc("eth_chainId", []), 16) == NETWORK["chainId"], "Wrong execution chain")
    require(rpc("eth_getBlockByNumber", ["0x0", False])["hash"] == NETWORK["genesisHash"], "Wrong execution genesis")
    finalized = rpc("eth_getBlockByNumber", ["finalized", False])
    import websocket
    from contextlib import closing
    with closing(websocket.create_connection(NETWORK["executionWss"], timeout=45)) as connection:
        connection.send(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "eth_getBlockByNumber", "params": [finalized["number"], False]}))
        reply = json.loads(connection.recv())
    require(reply.get("id") == 1 and reply.get("result", {}).get("hash") == finalized["hash"], "HTTP/WebSocket canonical disagreement")
    genesis = request(NETWORK["beaconHttp"] + "/eth/v1/beacon/genesis")["data"]
    require(genesis["genesis_validators_root"] == NETWORK["beaconGenesisValidatorsRoot"], "Wrong Beacon network")
    header = request(NETWORK["beaconHttp"] + "/eth/v1/beacon/headers/finalized")
    require(header["execution_optimistic"] is False and header["data"]["canonical"] is True, "Beacon finality is optimistic or noncanonical")
    block = request(NETWORK["beaconHttp"] + "/eth/v2/beacon/blocks/" + header["data"]["root"])
    require(block["execution_optimistic"] is False and block["version"].lower() == "fulu",
            "Finalized Beacon block is optimistic or uses an unqualified fork")
    payload = block["data"]["message"]["body"]["execution_payload"]
    height = int(payload["block_number"])
    require(rpc("eth_getBlockByNumber", [hex(height), False])["hash"] == payload["block_hash"]
            and height <= int(finalized["number"], 16), "Beacon/execution finalized view disagreement")
    for name in ("hoodi/keys", "hoodi/gear-keys", "source-chain/setup", "source-chain/alice", "source-chain/bob",
                 "supervisors", "follower", "inbound", "outbound/journal", "qualification"):
        (run / name).mkdir(mode=0o700, parents=True, exist_ok=True)
    save(run / "hoodi/network-gate.json", {**NETWORK, "status": "VERIFIED FOR TEST-ONLY SETUP",
         "finalizedBlock": height, "finalizedHash": payload["block_hash"], "beaconRoot": header["data"]["root"],
         "beaconSlot": int(header["data"]["header"]["message"]["slot"]), "testOnly": True})
    verification = json.loads(checked_file("verification.json").read_text())
    require(verification["status"] == "VERIFIED" and verification["checks"]
            and all(check["exitCode"] == 0 for check in verification["checks"]), "Bundle lacks successful component evidence")
    save(run / "qualification/components.json", {"status": "VERIFIED", "bundleSha256": bundle_hash,
         "verificationSha256": MANIFEST["files"]["verification.json"], "checks": verification["checks"]})
    roles = {}
    for role in ("checkpoint", "inbound", "campaign", "governance", "rotation", "paid", "initialization"):
        phrase = Keypair.generate_mnemonic()
        pair = Keypair.create_from_uri(phrase, ss58_format=42)
        path = run / "hoodi/gear-keys" / (role + ".suri")
        with path.open("x") as stream:
            stream.write(phrase + "\n")
            stream.flush()
            os.fsync(stream.fileno())
        roles[role] = {"ss58Address": pair.ss58_address, "publicKey": "0x" + pair.public_key.hex()}
    initialization = roles.pop("initialization")
    require(len({role["publicKey"] for role in roles.values()} | {initialization["publicKey"]}) == 7, "Repeated Gear identities")
    save(run / "hoodi/gear-addresses.json", {"roles": roles, "initialization": initialization, "testOnly": True})
    shutil.copytree(bundle / "ethereum", run / "forge-final")
    for path in (run / "forge-final").rglob("*"):
        path.chmod(0o700 if path.is_dir() else 0o600)
    (run / "forge-final").chmod(0o700)
    cache = run / "forge-final/cache/solidity-files-cache.json"
    save(cache, relocated_compiler_cache(json.loads(cache.read_text()),
         MANIFEST["solidity"]["compilerProjectRoot"], run / "forge-final"))
    for directory in (run / "hoodi/gear-keys", run, run.parent):
        fd = os.open(directory, os.O_RDONLY)
        os.fsync(fd)
        os.close(fd)
    save(run / "run-prepared.json", {"phase": "prepared", "runId": run_id, "bundleSha256": bundle_hash,
         "testOnly": True, "funding": "NOT STARTED", "deployment": "NOT STARTED"})
    print(json.dumps({"run": str(run), "bundleSha256": bundle_hash, "phase": "prepared", "testOnly": True}))


if __name__ == "__main__":
    main()
