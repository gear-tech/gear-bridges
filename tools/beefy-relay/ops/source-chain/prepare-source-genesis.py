#!/usr/bin/env python3
"""Generate this run's fresh native two-validator Hoodi-bound chain spec."""
from __future__ import annotations

import hashlib
import json
import os
import re
import shutil
import subprocess
from pathlib import Path

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from run_context import RUN, artifact, MANIFEST, CONFIG
OUT = RUN / "source-chain"
IDENTITY = OUT / "identity.json"
MARKER = RUN / "hoodi" / "funding-complete.json"
ROLE_FILE = RUN / "hoodi" / "gear-addresses.json"
GEAR = artifact("gear")
CHAIN_ID = 560048
ALICE = "5GrwvaEF5zXb26Fz9rcQpDWS57CtERHpNehXCPcNoHGKutQY"
MASK64 = (1 << 64) - 1
P1, P2, P3, P4, P5 = (
    11400714785074694791, 14029467366897019727, 1609587929392839161,
    9650029242287828579, 2870177450012600261,
)


def fail(message: str) -> None:
    raise SystemExit(message)


def write_once(path: Path, data: bytes) -> None:
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        if path.read_bytes() != data:
            fail(f"refusing to replace pinned artifact: {path}")
        return
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    except BaseException:
        path.unlink(missing_ok=True)
        raise


def field(mapping: dict, name: str) -> str:
    normalize = lambda value: re.sub(r"[^a-z0-9]", "", value.lower())
    found = [key for key in mapping if normalize(key) == normalize(name)]
    if len(found) != 1:
        fail(f"expected exactly one {name} field; found {len(found)}")
    return found[0]


def twox64(data: bytes, seed: int) -> int:
    def rotl(value: int, count: int) -> int:
        return ((value << count) | (value >> (64 - count))) & MASK64

    def round_(acc: int, lane: int) -> int:
        acc = (acc + lane * P2) & MASK64
        return (rotl(acc, 31) * P1) & MASK64

    def merge(acc: int, value: int) -> int:
        acc ^= round_(0, value)
        return (acc * P1 + P4) & MASK64

    size, offset = len(data), 0
    if size >= 32:
        v1, v2 = (seed + P1 + P2) & MASK64, (seed + P2) & MASK64
        v3, v4 = seed & MASK64, (seed - P1) & MASK64
        while offset <= size - 32:
            v1 = round_(v1, int.from_bytes(data[offset:offset + 8], "little")); offset += 8
            v2 = round_(v2, int.from_bytes(data[offset:offset + 8], "little")); offset += 8
            v3 = round_(v3, int.from_bytes(data[offset:offset + 8], "little")); offset += 8
            v4 = round_(v4, int.from_bytes(data[offset:offset + 8], "little")); offset += 8
        result = (rotl(v1, 1) + rotl(v2, 7) + rotl(v3, 12) + rotl(v4, 18)) & MASK64
        result = merge(merge(merge(merge(result, v1), v2), v3), v4)
    else:
        result = (seed + P5) & MASK64
    result = (result + size) & MASK64
    while offset <= size - 8:
        lane = round_(0, int.from_bytes(data[offset:offset + 8], "little"))
        result ^= lane
        result = (rotl(result, 27) * P1 + P4) & MASK64
        offset += 8
    if offset <= size - 4:
        result ^= (int.from_bytes(data[offset:offset + 4], "little") * P1) & MASK64
        result = (rotl(result, 23) * P2 + P3) & MASK64
        offset += 4
    while offset < size:
        result ^= (data[offset] * P5) & MASK64
        result = (rotl(result, 11) * P1) & MASK64
        offset += 1
    result ^= result >> 33
    result = (result * P2) & MASK64
    result ^= result >> 29
    result = (result * P3) & MASK64
    return result ^ (result >> 32)


def twox128(data: bytes) -> bytes:
    return twox64(data, 0).to_bytes(8, "little") + twox64(data, 1).to_bytes(8, "little")


def rtk_run(args: list[str]) -> str:
    rtk = shutil.which("rtk")
    if not rtk:
        fail("rtk must be available on PATH")
    result = subprocess.run([rtk, "proxy", *map(str, args)], capture_output=True, text=True)
    if result.returncode:
        fail(f"command failed ({result.returncode}): {' '.join(map(str, args))}\n{result.stderr.strip()}")
    return result.stdout


def main() -> None:
    identity = json.loads(IDENTITY.read_text())
    marker_bytes = MARKER.read_bytes()
    marker = json.loads(marker_bytes)
    if hashlib.sha256(marker_bytes).hexdigest() != identity["funding"]["markerSha256"]:
        fail("funding marker changed after nonce/domain pinning")
    if marker.get("phase") != "finalized" or marker.get("chainId") != CHAIN_ID:
        fail("funding-complete.json is not finalized for Hoodi")
    if marker.get("roles", {}).get("deployer", "").lower() != identity["deployerAddress"].lower():
        fail("funding marker deployer differs from pinned source identity")
    if marker.get("finalizedBlock") != identity["funding"]["finalizedBlock"] or marker.get("finalizedHash") != identity["funding"]["finalizedHash"]:
        fail("funding marker changed after nonce/domain pinning")
    binary_hash = hashlib.sha256()
    with GEAR.open("rb") as binary:
        for chunk in iter(lambda: binary.read(1024 * 1024), b""):
            binary_hash.update(chunk)
    actual_binary = binary_hash.hexdigest()
    if actual_binary != MANIFEST["files"][MANIFEST["binaries"]["gear"]]:
        fail("native Gear executable differs from the sealed bundle hash")
    roles = json.loads(ROLE_FILE.read_text()).get("roles", {})
    expected_roles = {"checkpoint", "inbound", "campaign", "governance", "rotation", "paid"}
    if set(roles) != expected_roles:
        fail("gear-addresses.json does not contain the exact six fresh role identities")
    addresses = [entry.get("ss58Address") for entry in roles.values()]
    if any(not isinstance(address, str) or not address for address in addresses) or len(set(addresses)) != 6:
        fail("fresh Gear role identities are missing or duplicated")

    spec = json.loads(rtk_run([GEAR, "build-spec", "--chain", "local", "--disable-default-bootnode"]))
    if spec.get("id") != "vara_local_testnet":
        fail(f"unexpected native local chain spec id {spec.get('id')!r}")
    patch = spec["genesis"]["runtimeGenesis"]["patch"]
    session = patch[field(patch, "session")]
    staking = patch[field(patch, "staking")]
    if len(session[field(session, "keys")]) != 2 or len(staking[field(staking, "invulnerables")]) != 2:
        fail("fresh local genesis must retain exactly two Alice/Bob authorities")
    balances = patch[field(patch, "balances")]
    rows = balances[field(balances, "balances")]
    alice_rows = [row for row in rows if row[0] == ALICE]
    if len(alice_rows) != 1 or not isinstance(alice_rows[0][1], int):
        fail("native local genesis has no unique endowed Alice account")
    endowment = alice_rows[0][1]
    existing = {row[0]: row[1] for row in rows}
    for address in addresses:
        if address in existing:
            if existing[address] != endowment:
                fail(f"existing genesis balance conflicts for fresh role {address}")
        else:
            rows.append([address, endowment])

    bridge = patch[field(patch, "gearEthBridge")]
    bridge_key = field(bridge, "bridgeDomain")
    bridge[bridge_key] = identity["bridgeDomain"]
    profile = CONFIG.get("runtimeProfile")
    if profile is not None:
        beefy = patch[field(patch, "beefy")]
        beefy[field(beefy, "genesisBlock")] = None
    run_id = identity["runId"]
    spec["name"] = f"Vara BEEFY Hoodi {run_id[:8]}"
    spec["id"] = f"vara-beefy-hoodi-{run_id[:8]}"
    spec["bootNodes"] = []
    structured = (json.dumps(spec, sort_keys=True, indent=2) + "\n").encode()
    structured_path = OUT / "chain-spec-structured.json"
    write_once(structured_path, structured)

    raw_text = rtk_run([GEAR, "build-spec", "--chain", structured_path, "--disable-default-bootnode", "--raw"])
    raw_spec = json.loads(raw_text)
    top = raw_spec["genesis"]["raw"]["top"]
    domain_key = "0x" + (twox128(b"GearEthBridge") + twox128(b"BridgeDomain")).hex()
    raw_domain = top.get(domain_key)
    if not isinstance(raw_domain, str) or raw_domain.lower() != identity["bridgeDomain"].lower():
        fail("raw chain spec does not contain the pinned GearEthBridge::BridgeDomain")
    code = top.get("0x3a636f6465")
    if not isinstance(code, str) or not code.startswith("0x"):
        fail("raw chain spec is missing runtime :code")
    if profile is not None:
        genesis_key = "0x" + (twox128(b"Beefy") + twox128(b"GenesisBlock")).hex()
        if top.get(genesis_key) != "0x00":
            fail("normal source must start with inactive BEEFY for its separate governed activation")
        from Crypto.Hash import keccak
        code_bytes = bytes.fromhex(code[2:])
        hashes = {"runtimeCodeSha256": hashlib.sha256(code_bytes).hexdigest(),
                  "runtimeCodeBlake2b256": hashlib.blake2b(code_bytes, digest_size=32).hexdigest(),
                  "runtimeCodeKeccak256": keccak.new(digest_bits=256, data=code_bytes).hexdigest()}
        if any(profile[name].removeprefix("0x") != value for name, value in hashes.items()):
            fail("native build-spec runtime differs from independently approved PR5642 artifact; HOLD")
        raw_spec["runtimeProfile"] = profile
    raw = (json.dumps(raw_spec, sort_keys=True, indent=2) + "\n").encode()
    raw_path = OUT / "chain.raw.json"
    write_once(raw_path, raw)
    evidence = {
        "schemaVersion": 1,
        "runId": run_id,
        "chainId": spec["id"],
        "destinationChainId": CHAIN_ID,
        "destinationQueue": identity["destinationQueue"],
        "deployerAddress": identity["deployerAddress"],
        "reservedNonce": identity["nonce"]["value"],
        "messageQueueCreateNonce": identity["messageQueueCreateNonce"],
        "sourceDomain": identity["sourceDomain"],
        "bridgeDomain": identity["bridgeDomain"],
        "bridgeDomainPreimage": identity["bridgeDomainPreimage"],
        "validatorCount": 2,
        "aliceEndowment": str(endowment),
        "roleEndowments": {role: {"ss58Address": roles[role]["ss58Address"], "amount": str(endowment)} for role in sorted(roles)},
        "structuredSpecPath": str(structured_path),
        "rawSpecPath": str(raw_path),
        "structuredSpecSha256": hashlib.sha256(structured).hexdigest(),
        "rawSpecSha256": hashlib.sha256(raw).hexdigest(),
        "runtimeCodeSha256": hashlib.sha256(bytes.fromhex(code[2:])).hexdigest(),
        "gearBinarySha256": actual_binary,
        "builderCommands": [
            ["rtk", "proxy", str(GEAR), "build-spec", "--chain", "local", "--disable-default-bootnode"],
            ["rtk", "proxy", str(GEAR), "build-spec", "--chain", str(structured_path), "--disable-default-bootnode", "--raw"],
        ],
        "fundingFinality": {"block": marker["finalizedBlock"], "hash": marker["finalizedHash"]},
    }
    if profile is not None:
        evidence["runtimeProfile"] = profile
    evidence_path = OUT / "spec-evidence.json"
    write_once(evidence_path, (json.dumps(evidence, sort_keys=True, indent=2) + "\n").encode())
    print(json.dumps({"structuredSpec": str(structured_path), "rawSpec": str(raw_path), "evidence": str(evidence_path), "validatorCount": 2, "roleEndowment": str(endowment), "structuredSpecSha256": evidence["structuredSpecSha256"], "rawSpecSha256": evidence["rawSpecSha256"], "runtimeCodeSha256": evidence["runtimeCodeSha256"], "gearBinarySha256": actual_binary}, sort_keys=True))


if __name__ == "__main__":
    main()
