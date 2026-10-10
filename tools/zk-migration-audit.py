#!/usr/bin/env python3
"""Offline consistency checks and unsigned ZK-to-BEEFY proposal preparation.

No RPC, signer, environment credentials, or filesystem writes. A consistent
manifest is not independent chain verification or permission to migrate.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys


BINDINGS = (
    "elChainId elGenesis elFinalizedHash elFinalizedNumber gearGenesis "
    "gearFinalizedHash gearFinalizedNumber ethereumDiscoveryStartBlock queue "
    "queueImplementation queueImplementationCodeHash oldVerifier oldVerifierCodeHash "
    "manager managerQueue queueAdmin queuePauser managerAdmin managerPauser "
    "adminSource pauserSource bridgeAdmin bridgePauser vftManagers erc20Tokens queuePaused "
    "managerPaused sourcePaused candidateSourceGenesis candidateDestinationChainId "
    "candidateQueue candidateVerifier candidateSourceDomain candidateBridgeDomain "
    "candidateMmrStartBlock candidateLatestBeefyBlock candidateMMRRoot candidateLive "
    "releaseImplementation releaseCodeHash"
).split()
ASSET_FIELDS = (
    "erc20 gearToken tokenType supplyType decimalsEth decimalsGear consumer "
    "gearMinter gearBurner gearAdmin gearPauser wrappedSupplyRaw"
).split()
ARTIFACTS = (
    "authority storageLayout sourceActivation replayContinuity oldProofIndex "
    "workerJournalIndex rehearsal releaseAbi ledger accounting"
).split()
ADDRESS = re.compile(r"0x[0-9a-fA-F]{40}\Z")
HASH = re.compile(r"0x[0-9a-fA-F]{64}\Z")
HEX_BYTES = re.compile(r"0x(?:[0-9a-fA-F]{2})*\Z")


class Blocked(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise Blocked(message)


def uint(value, name):
    require(type(value) is int or isinstance(value, str), f"{name}: integer required")
    if isinstance(value, str):
        require(re.fullmatch(r"(?:0x[0-9a-fA-F]+|[0-9]+)", value), f"{name}: invalid integer")
        value = int(value, 16 if value.startswith("0x") else 10)
    require(value >= 0, f"{name}: negative integer")
    return value


def measured(inventory, pointer, deployment):
    require(isinstance(pointer, str), "missing inventory JSON pointer")
    require(pointer.startswith(f"/deployments/{deployment}/"), "binding escapes selected deployment")
    value, datum = inventory, None
    try:
        for key in pointer.split("/")[1:]:
            key = key.replace("~1", "/").replace("~0", "~")
            if isinstance(value, dict) and "status" in value and "value" in value:
                datum = value if key == "value" else None
            value = value[int(key)] if isinstance(value, list) else value[key]
    except (KeyError, IndexError, TypeError, ValueError) as error:
        raise Blocked(f"missing measured datum: {pointer}") from error
    if isinstance(value, dict) and "status" in value and "value" in value:
        datum, value = value, value["value"]
    require(datum is not None and datum.get("status") == "measured", f"unmeasured datum: {pointer}")
    require(value is not None and datum.get("provenance") and datum.get("observedAt")
            and datum.get("snapshot"), f"incomplete provenance/snapshot: {pointer}")
    return value


def artifact(plan, name, base):
    item = plan.get("artifacts", {}).get(name, {})
    require(isinstance(item.get("path"), str) and item["path"], f"missing artifact: {name}")
    path = (base / item["path"]).resolve()
    # Reports/ABI/ledger JSON only; never read key files or signed transaction bodies.
    require(path.suffix == ".json" and not any(
        word in path.name.lower() for word in ("secret", "private", "signed", "keyfile")
    ), f"{name}: non-secret JSON evidence required")
    data = path.read_bytes()
    require(hashlib.sha256(data).hexdigest() == item.get("sha256"), f"artifact hash mismatch: {name}")
    return json.loads(data)


def check_identity(bindings, plan):
    b = bindings
    for name in ("queue", "queueImplementation", "oldVerifier", "manager", "managerQueue",
                 "queueAdmin", "queuePauser", "managerAdmin", "managerPauser",
                 "candidateQueue", "candidateVerifier", "releaseImplementation"):
        require(ADDRESS.fullmatch(b[name]) and int(b[name], 16) != 0, f"{name}: nonzero address required")
    for name in ("elGenesis", "elFinalizedHash", "gearGenesis", "gearFinalizedHash",
                 "queueImplementationCodeHash", "oldVerifierCodeHash", "adminSource", "pauserSource",
                 "bridgeAdmin", "bridgePauser", "candidateSourceGenesis", "candidateSourceDomain",
                 "candidateBridgeDomain", "candidateMMRRoot", "releaseCodeHash"):
        require(HASH.fullmatch(b[name]) and int(b[name], 16) != 0, f"{name}: nonzero 32-byte value required")
    for left, right in (("managerQueue", "queue"), ("queueAdmin", "managerAdmin"),
                        ("queuePauser", "managerPauser"), ("adminSource", "bridgeAdmin"),
                        ("pauserSource", "bridgePauser"), ("candidateQueue", "queue"),
                        ("candidateSourceGenesis", "gearGenesis")):
        require(b[left].lower() == b[right].lower(), f"identity mismatch: {left}/{right}")
    require(uint(b["elChainId"], "elChainId") == uint(b["candidateDestinationChainId"], "candidateDestinationChainId"),
            "candidate destination chain differs from custody chain")
    require(b["candidateVerifier"].lower() != b["oldVerifier"].lower(), "verifier not changed")
    require(b["candidateLive"] is True, "candidate not live at inventory snapshot")
    managers = b["vftManagers"]
    require(isinstance(managers, list) and managers and all(HASH.fullmatch(m) for m in managers),
            "missing authorized historical VFT manager set")
    require(len(set(m.lower() for m in managers)) == len(managers), "duplicate VFT managers")
    tokens = b["erc20Tokens"]
    require(isinstance(tokens, list) and tokens and all(ADDRESS.fullmatch(token) for token in tokens),
            "missing complete registered ERC20 set")
    require(len({token.lower() for token in tokens}) == len(tokens), "duplicate registered ERC20")
    expected = {key: b[key] for key in ("queue", "manager", "gearGenesis", "vftManagers")}
    replay_pin = plan.get("artifacts", {}).get("replayContinuity", {}).get("sha256")
    require(isinstance(replay_pin, str) and re.fullmatch(r"[0-9a-f]{64}", replay_pin),
            "missing original replay-state evidence pin")
    expected["replayContinuitySha256"] = replay_pin
    require(plan.get("preserve") == expected, "custody/source/replay-owner identity would change")
    require(all(b[name] is True for name in ("queuePaused", "managerPaused", "sourcePaused")),
            "finalized cutover snapshot is not frozen")
    c = plan.get("cutover", {})
    for name in ("assetFreezeBlock", "legacyRootCeiling", "newRootMinimum", "nextNonceAtAssetFreeze", "executionStopBlock"):
        require(name in c, f"missing watermark: {name}")
        c[name] = uint(c[name], name)
    require(c["assetFreezeBlock"] <= c["legacyRootCeiling"] < 2**32 - 1, "invalid legacy block boundary")
    require(c["newRootMinimum"] == c["legacyRootCeiling"] + 1, "root ranges overlap or leave a gap")
    require(c["legacyRootCeiling"] <= uint(b["gearFinalizedNumber"], "gearFinalizedNumber"), "source boundary not finalized")
    require(c["executionStopBlock"] <= uint(b["elFinalizedNumber"], "elFinalizedNumber"), "execution stop not finalized")
    require(0 < uint(b["candidateMmrStartBlock"], "candidateMmrStartBlock") <= c["newRootMinimum"]
            < uint(b["candidateLatestBeefyBlock"], "candidateLatestBeefyBlock"), "MMR cannot prove first post-cutover root")


def check_ledgers(bindings, cutover, assets, ledger, accounting):
    require(assets, "complete exact asset mappings required")
    pairs = {(row["erc20"].lower(), row["gearToken"].lower()) for row in assets}
    require(len(pairs) == len(assets)
            and len({pair[0] for pair in pairs}) == len(pairs)
            and len({pair[1] for pair in pairs}) == len(pairs), "duplicate or nonexclusive active asset mapping")
    require({pair[0] for pair in pairs} == {token.lower() for token in bindings["erc20Tokens"]}, "asset package omits or adds a registered ERC20")
    for row in assets:
        require(ADDRESS.fullmatch(row["erc20"]) and HASH.fullmatch(row["gearToken"]), "invalid asset identity")
        require(row["consumer"] in bindings["vftManagers"], "receipt consumer not an existing authorized manager")
        require((uint(row["tokenType"], "tokenType"), uint(row["supplyType"], "supplyType")) in ((1, 0), (2, 1)),
                "Ethereum/Gear origin mapping disagrees")
        require(uint(row["decimalsEth"], "decimalsEth") == uint(row["decimalsGear"], "decimalsGear") <= 255, "asset decimal mismatch")
        if uint(row["tokenType"], "tokenType") == 1:
            require(row["gearMinter"] == row["consumer"] and row["gearBurner"] == row["consumer"], "wrapped token has a different mint/burn consumer")
    coverage = ledger.get("coverage", {})
    require(coverage.get("completeFromOrigin") is True, "origin-through-freeze ledgers absent/incomplete")
    require(coverage.get("ethereumStartBlock") == uint(bindings["ethereumDiscoveryStartBlock"], "ethereumDiscoveryStartBlock")
            and coverage.get("ethereumThroughBlock") == cutover["executionStopBlock"]
            and coverage.get("sourceThroughBlock") == cutover["legacyRootCeiling"], "ledger watermarks disagree")
    require(ledger.get("gearGenesis") == bindings["gearGenesis"] and ledger.get("queue") == bindings["queue"]
            and ledger.get("manager") == bindings["manager"], "ledger belongs to another replay/custody namespace")
    require(ledger.get("unresolved") == [], "ambiguous, reserved, partial, missing-proof or control liabilities remain")
    claims = ledger.get("claims")
    require(isinstance(claims, list), "missing claim ledger (empty must be explicitly proven)")
    keys, consumers = set(), {}
    for claim in claims:
        key = json.dumps([claim.get("direction"), claim.get("key")], sort_keys=True)
        require(claim.get("key") is not None and key not in keys, "duplicate or missing original claim identity")
        keys.add(key)
        if claim.get("direction") == "ethToGear":
            require(claim.get("status") == "processed", "unsettled inbound receipt cannot cross cutover")
            receipt = tuple(claim.get("receiptKey", []))
            require(len(receipt) == 2 and claim.get("consumer") in bindings["vftManagers"], "missing original receipt binding")
            require(len(claim["key"]) == 4 and claim["key"][0] == uint(bindings["elChainId"], "elChainId")
                    and claim["key"][1] == bindings["manager"] and HASH.fullmatch(claim["key"][2])
                    and all(uint(part, "receiptKey") < 2**64 for part in receipt), "receipt namespace changed")
            require(consumers.setdefault(receipt, claim["consumer"]) == claim["consumer"], "receipt has two consumers")
        else:
            require(claim.get("direction") == "gearToEth" and claim.get("status") in ("released", "registered", "refundedNoQueue"),
                    "unclassified outbound liability")
            require(claim["key"][0] == bindings["gearGenesis"], "outbound source namespace changed")
            if claim["status"] != "refundedNoQueue":
                require(len(claim["key"]) == 2
                        and uint(claim["key"][1], "messageNonce") < cutover["nextNonceAtAssetFreeze"], "outbound claim crosses asset watermark")
            if claim["status"] == "registered":
                require(claim.get("root") and claim.get("rootTimestamp") is not None and claim.get("proofArtifact"),
                        "old redemption lacks original registered root/maturity/proof")
        require(claim.get("evidence"), "claim lacks canonical per-effect evidence")
    rows = accounting.get("rows", [])
    require({(r["erc20"].lower(), r["gearToken"].lower()) for r in rows} == pairs and len(rows) == len(pairs),
            "accounting does not cover exact asset set")
    for row in rows:
        quantities = []
        for name in ("escrowRaw", "wrappedSupplyRaw", "inboundPendingRaw", "outboundPendingRaw", "provenSurplusRaw"):
            value = row.get(name)
            require(isinstance(value, str) and re.fullmatch(r"[0-9]+", value), f"{name}: decimal raw-unit string required")
            quantities.append(int(value))
        escrow, supply, inbound, outbound, surplus = quantities
        observed = next(asset for asset in assets if asset["erc20"].lower() == row["erc20"].lower())
        require(escrow == uint(observed["escrowRaw"], "observed escrow")
                and supply == uint(observed["wrappedSupplyRaw"], "observed wrapped supply"), "accounting changed observed custody or supply")
        require(escrow == supply + inbound + outbound + surplus, f"unreconciled raw-unit liability: {row['erc20']}")
        require(inbound == 0, "inbound drain is incomplete")
        require(surplus == 0 or row.get("surplusEvidence"), "unexplained surplus cannot conceal imbalance")


def cast(*arguments):
    return subprocess.check_output(["cast", *arguments], text=True, timeout=20).strip()


def _precutover_proposals(b, data):
    proposals = []
    for action, source, destination, payload in (
        ("pauseCustodyIngress", b["pauserSource"], b["queuePauser"], "0x01" + b["manager"][2:]),
        ("pauseQueueUsers", b["pauserSource"], b["queuePauser"], "0x01" + b["queue"][2:]),
        ("upgradeQueueInPlace", b["adminSource"], b["queueAdmin"], "0x03" + b["queue"][2:] + b["releaseImplementation"][2:] + data[2:]),
    ):
        require(HEX_BYTES.fullmatch(payload), "malformed packed governance payload")
        proposals.append({"action": action, "chain": "gear", "requiredEffectiveOrigin": source,
                          "call": "GearEthBridge.send_eth_message", "args": {"destination": destination, "payload": payload},
                          "authorization": "UNSIGNED; pinned metadata/proxy-or-root dispatch authority required"})
    return proposals


def prepare(inventory_bytes, plan, deployment, base):
    inventory = json.loads(inventory_bytes)
    require(isinstance(plan, dict), "cutover plan must be an object")
    require(plan.get("deployment") == deployment, "selected deployment differs from plan")
    require(plan.get("inventorySha256") == hashlib.sha256(inventory_bytes).hexdigest(), "inventory pin changed")
    b = {name: measured(inventory, plan.get("bindings", {}).get(name), deployment) for name in BINDINGS}
    check_identity(b, plan)
    assets = [{name: measured(inventory, row.get(name), deployment) for name in ASSET_FIELDS}
              for row in plan.get("assets", [])]
    for row, references in zip(assets, plan.get("assets", [])):
        parts = references.get("escrowParts", [])
        owners = [part["owner"].lower() for part in parts]
        expected_owners = [b["manager"].lower()] if uint(row["tokenType"], "tokenType") == 1 else [m.lower() for m in b["vftManagers"]]
        require(sorted(owners) == sorted(expected_owners), "custody observations omit or duplicate a historical owner")
        row["escrowRaw"] = str(sum(uint(measured(inventory, part.get("value"), deployment), "escrowRaw") for part in parts))
        row["wrappedSupplyRaw"] = str(uint(row["wrappedSupplyRaw"], "wrappedSupplyRaw"))
    evidence = {name: artifact(plan, name, base) for name in ARTIFACTS}
    check_ledgers(b, plan["cutover"], assets, evidence["ledger"], evidence["accounting"])
    chain = uint(b["elChainId"], "elChainId")
    require(chain < 2**256, "destination chain ID exceeds domain width")
    packed = "0x" + b["candidateSourceDomain"][2:] + f"{chain:064x}" + b["queue"][2:]
    domain = cast("keccak", "0x" + b"vara/gear-eth-bridge-domain/v2".hex() + packed[2:])
    require(domain.lower() == b["candidateBridgeDomain"].lower(), "candidate domain not bound to original queue/chain")
    init = plan.get("initializer", {})
    signature, argument_bindings = init.get("signature"), init.get("argumentBindings", [])
    abi = evidence["releaseAbi"]
    abi = abi.get("abi", []) if isinstance(abi, dict) else abi
    signatures = {f"{fn['name']}({','.join(i['type'] for i in fn['inputs'])})"
                  for fn in abi if fn.get("type") == "function"}
    require(signature in signatures, "missing audited executable initializer ABI (current queue has none)")
    require(all(name in argument_bindings for name in ("oldVerifier", "candidateVerifier", "cutover.legacyRootCeiling")),
            "initializer lacks old-verifier CAS/new-verifier/legacy-boundary bindings")
    arguments = [str(plan["cutover"][name.split(".", 1)[1]]) if name.startswith("cutover.") else str(b[name])
                 for name in argument_bindings]
    data = cast("calldata", signature, *arguments)
    require(data.lower() == str(init.get("calldata", "")).lower() and data != "0x", "initializer calldata differs from audited release")
    upgrade = cast("calldata", "upgradeToAndCall(address,bytes)", b["releaseImplementation"], data)
    proposals = _precutover_proposals(b, data)
    return {"manifestStatus": "CONSISTENT_FOR_UNSIGNED_REVIEW_ONLY", "executionAuthorized": False,
            "migrationReadiness": "BLOCKED_PENDING_INDEPENDENT_FINALIZED_REVALIDATION_AND_GOVERNANCE_APPROVAL",
            "evidenceLimit": "Measured/status/hash/coverage labels are inputs, not independent authentication, audit approval or proof of complete history.",
            "inventorySha256": plan["inventorySha256"], "deployment": deployment, "proposals": proposals,
            "atomicEthereumUpgradeSimulation": {"from": b["queueAdmin"], "to": b["queue"], "data": upgrade, "value": "0x0"},
            "sequencing": "Not a batch: only pause/upgrade proposals are supplied. Never prestage ordinary unpause or unguarded rollback. After finalized G4/G5 checks, authorize and queue fresh resume messages through the qualified new BEEFY root path."}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inventory", type=Path)
    parser.add_argument("--deployment", required=True, choices=("mainnet", "publicHoodi"))
    parser.add_argument("--plan", type=Path)
    args = parser.parse_args()
    try:
        inventory_bytes = args.inventory.read_bytes()
        inventory = json.loads(inventory_bytes)
        require(isinstance(inventory, dict) and args.deployment in inventory.get("deployments", {}),
                "missing selected deployment inventory")
        require(args.plan is not None, "missing separately reviewed cutover plan, release ABI, authorities and complete liability artifacts")
        result = prepare(inventory_bytes, json.loads(args.plan.read_bytes()), args.deployment, args.plan.parent)
    except (Blocked, OSError, ValueError, KeyError, TypeError, AttributeError, IndexError, subprocess.SubprocessError) as error:
        print(json.dumps({"manifestStatus": "BLOCKED", "executionAuthorized": False, "blocker": str(error)}, indent=2))
        return 2
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    sys.exit(main())
