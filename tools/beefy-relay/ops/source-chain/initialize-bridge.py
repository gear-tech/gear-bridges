#!/usr/bin/env python3
"""Activate this disposable source's genuine BEEFY session and bridge builtin."""
import argparse
import hashlib
import fcntl
import json
import os
import subprocess
import time
from pathlib import Path

from substrateinterface import Keypair, SubstrateInterface

import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from run_context import RUN, CONFIG, artifact, private_text, digest
SOURCE = RUN / "source-chain"
SETUP = SOURCE / "setup"
KEY = RUN / "hoodi" / "gear-keys" / "initialization.suri"
GEAR = str(artifact("gear"))
AMOUNT = 100_000_000_000_000


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def write_once(path, value):
    data = (json.dumps(value, sort_keys=True, indent=2) + chr(10)).encode()
    try:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    except FileExistsError:
        require(path.read_bytes() == data, f"refusing to change sealed evidence {path.name}")
        return
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    fd = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def rpc(api, method, params):
    response = api.rpc_request(method, params)
    require("error" not in response and "result" in response, f"RPC {method} failed; HOLD")
    return response["result"]


def finalized(api):
    block_hash = api.get_chain_finalised_head()
    return api.get_block_number(block_hash), block_hash


def state(api, name, block_hash=None):
    return api.query("GearEthBridge", name, block_hash=block_hash).value


def account(api, address, block_hash=None):
    return api.query("System", "Account", [address], block_hash=block_hash).value


def event_info(event):
    value = event.value
    phase = value.get("phase", {})
    index = phase.get("ApplyExtrinsic") if isinstance(phase, dict) else value.get("extrinsic_idx")
    event = value.get("event", {})
    return index, event.get("module_id"), event.get("event_id")


def scan(api, intent, head):
    target = intent["extrinsicHash"].lower()
    for number in range(intent["startFinalizedHeight"], head + 1):
        block_hash = api.get_block_hash(number)
        block = rpc(api, "chain_getBlock", [block_hash])
        for index, raw in enumerate(block["block"]["extrinsics"]):
            if "0x" + hashlib.blake2b(bytes.fromhex(raw[2:]), digest_size=32).hexdigest() != target:
                continue
            events = [(module, name) for event in api.get_events(block_hash)
                      for event_index, module, name in [event_info(event)] if event_index == index]
            require(("System", "ExtrinsicSuccess") in events, f"{intent['name']} finalized without ExtrinsicSuccess; HOLD")
            if intent["expectedEvent"]:
                require(tuple(intent["expectedEvent"]) in events, f"{intent['name']} finalized without expected event; HOLD")
            require(api.get_block_hash(number).lower() == block_hash.lower(), "source canonicality changed; HOLD")
            return {"name": intent["name"], "extrinsicHash": intent["extrinsicHash"], "block": number,
                    "blockHash": block_hash, "extrinsicIndex": index, "events": [".".join(e) for e in events]}
    return None


def submit_once(api, name, call, signer, genesis, expected_event=None):
    intent_file = SETUP / (name + ".intent.json")
    receipt_file = SETUP / (name + ".finalized.json")
    if intent_file.exists():
        intent = json.loads(intent_file.read_text())
        require(intent["name"] == name and intent["genesis"] == genesis and
                intent["signer"] == signer.ss58_address and intent["expectedEvent"] == expected_event
                and intent["call"] == str(call.data),
                f"{name} original signed transaction identity differs; HOLD")
    else:
        height, block_hash = finalized(api)
        nonce = api.get_account_nonce(signer.ss58_address)
        require(nonce == int(account(api, signer.ss58_address, block_hash)["nonce"]),
                f"{name} signer has an unresolved transaction; HOLD")
        signed = api.create_signed_extrinsic(call, signer, nonce=nonce)
        raw = str(signed.data)
        tx_hash = "0x" + hashlib.blake2b(bytes.fromhex(raw[2:]), digest_size=32).hexdigest()
        require(tx_hash.lower() == "0x" + signed.extrinsic_hash.hex(), "signed Gear transaction hash mismatch")
        intent = {"name": name, "genesis": genesis, "signer": signer.ss58_address, "nonce": nonce,
                  "startFinalizedHeight": height, "extrinsicHash": tx_hash, "raw": raw,
                  "expectedEvent": expected_event, "call": str(call.data)}
        write_once(intent_file, intent)  # Signed bytes/hash survive every interruption before broadcast.
    require(intent["raw"].lower().endswith(intent["call"][2:].lower()), "original signed call differs; HOLD")
    require("0x" + hashlib.blake2b(bytes.fromhex(intent["raw"][2:]), digest_size=32).hexdigest() == intent["extrinsicHash"],
            f"{name} signed journal was corrupted; HOLD")
    if receipt_file.exists():
        stored = json.loads(receipt_file.read_text())
        require(api.get_block_hash(stored["block"]).lower() == stored["blockHash"].lower(),
                f"{name} finalized receipt is noncanonical; HOLD")
        require(scan(api, intent, stored["block"]) == stored, f"{name} original receipt differs; HOLD")
        return stored
    head, block_hash = finalized(api)
    receipt = scan(api, intent, head)
    if receipt is None:
        chain_nonce = int(account(api, signer.ss58_address, block_hash)["nonce"])
        require(chain_nonce <= intent["nonce"], f"{name} nonce spent without original canonical receipt; HOLD")
        require(chain_nonce == intent["nonce"], f"{name} original nonce not available; HOLD")
        response = rpc(api, "author_submitExtrinsic", [intent["raw"]])
        require(response.lower() == intent["extrinsicHash"].lower(), f"{name} RPC hash differs from original signed bytes; HOLD")
        deadline = time.monotonic() + 180
        previous = head
        while time.monotonic() < deadline:
            head, _ = finalized(api)
            if head > previous:
                receipt = scan(api, intent, head)
                if receipt:
                    break
                previous = head
            time.sleep(2)
        require(receipt is not None, f"{name} original transaction outcome unknown; HOLD")
    write_once(receipt_file, receipt)
    print(json.dumps({k: receipt[k] for k in ("name", "extrinsicHash", "block", "blockHash", "events")}, sort_keys=True), flush=True)
    return receipt


def verify_prefunded(api, evidence, genesis):
    require(evidence["kind"] == "prefunded" and evidence["genesis"] == genesis, "wrong prefunding identity")
    require(api.get_block_hash(evidence["block"]) == evidence["blockHash"], "prefunding block became noncanonical")
    require(api.get_block_number(api.get_chain_finalised_head()) >= evidence["block"], "prefunding is not finalized")
    data = api.query("System", "Account", [evidence["account"]], block_hash=evidence["blockHash"]).value["data"]
    require(str(data["free"]) == evidence["free"] and str(data["frozen"]) == evidence["frozen"]
            and int(data["free"]) - int(data["frozen"]) >= AMOUNT // 2, "prefunding balance evidence mismatch")


def profiled_source_pin(identity):
    """Authenticate preactivation state without treating an RPC as artifact approval."""
    profile = CONFIG["runtimeProfile"]
    spec = json.loads((SOURCE / "spec-evidence.json").read_text())
    require(identity.get("runtimeProfile") == spec.get("runtimeProfile") == profile
            and digest(SOURCE / "chain.raw.json") == spec["rawSpecSha256"], "Normal source artifacts changed")
    apis = [SubstrateInterface(url=CONFIG["source"][node + "Rpc"], ss58_format=42) for node in ("alice", "bob")]
    genesis = [api.get_block_hash(0) for api in apis]
    require(genesis[0] == genesis[1], "Normal source genesis differs across witnesses")
    height = min(finalized(api)[0] for api in apis)
    pins = [api.get_block_hash(height) for api in apis]
    require(pins[0] == pins[1], "Normal source common finalized hashes differ")
    for api, pin in zip(apis, pins):
        code = bytes.fromhex(rpc(api, "state_getStorage", ["0x3a636f6465", pin])[2:])
        from Crypto.Hash import keccak
        require(hashlib.sha256(code).hexdigest() == profile["runtimeCodeSha256"].removeprefix("0x")
                and hashlib.blake2b(code, digest_size=32).hexdigest() == profile["runtimeCodeBlake2b256"].removeprefix("0x")
                and keccak.new(digest_bits=256, data=code).hexdigest() == profile["runtimeCodeKeccak256"].removeprefix("0x"),
                "Common finalized runtime differs from independent approved artifact; HOLD")
        babe = bytes.fromhex(rpc(api, "state_call", ["BabeApi_configuration", "0x", pin])[2:])
        require(len(babe) >= 16 and int.from_bytes(babe[:8], "little") == profile["slotDurationMs"]
                and int.from_bytes(babe[8:16], "little") == profile["epochDurationBlocks"], "Source cadence differs from the immutable profile; never change retained chain cadence")
        require(state(api, "BridgeDomain", pin) == identity["bridgeDomain"], "Pinned queue domain differs")
    return apis[0], {"phase": "preactivation-verified", "aliceRpc": CONFIG["source"]["aliceRpc"], "readiness": {"genesisHash": genesis[0], "commonFinalized": {"height": height, "hash": pins[0]}}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--stage", choices=("prepare-keys", "activate-beefy", "unpause"))
    parser.add_argument("--activation-delay-blocks", type=int)
    args = parser.parse_args()
    normal = CONFIG.get("runtimeProfile") is not None
    require(normal == (args.stage is not None), "Normal runtime requires separate explicit key/Root activation/unpause stages; legacy keeps its original flow")
    require((args.stage == "activate-beefy" and args.activation_delay_blocks is not None
             and 1 <= args.activation_delay_blocks < 2**32)
            or (args.stage != "activate-beefy" and args.activation_delay_blocks is None), "Explicit nonzero BEEFY activation delay required only at activation")
    os.umask(0o077)
    SETUP.mkdir(mode=0o700, exist_ok=True)
    SETUP.chmod(0o700)
    lock = (SETUP / "initialization.lock").open("a")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    identity = json.loads((SOURCE / "identity.json").read_text())
    if normal:
        api, launch = profiled_source_pin(identity)
        if args.stage != "prepare-keys":
            require((SETUP / "rotate-alice.finalized.json").exists(), "Prepare and finalize original keys before governed activation")
    else:
        launch = json.loads((SOURCE / "launch-state.json").read_text())
        require(launch["phase"] == "ready" and launch["aliceRpc"].startswith("ws://127.0.0.1:"), "fresh local source readiness required")
        api = SubstrateInterface(url=launch["aliceRpc"], ss58_format=42)
    genesis = api.get_block_hash(0)
    require(genesis == launch["readiness"]["genesisHash"] and state(api, "BridgeDomain") == identity["bridgeDomain"], "source genesis/domain mismatch; no Gear transactions sent")
    setup_suri = private_text(KEY)
    require(setup_suri != private_text(RUN / "hoodi" / "gear-keys" / "rotation.suri"), "warmup rotation key reserved")
    key_inspect = subprocess.run(["rtk", "proxy", GEAR, "key", "inspect", "--scheme", "ecdsa", "--output-type", "json", str(KEY)], capture_output=True, text=True)
    require(key_inspect.returncode == 0, "native Gear cannot derive setup BEEFY public key")
    beefy_key = json.loads(key_inspect.stdout)["publicKey"]
    require(len(bytes.fromhex(beefy_key[2:])) == 33, "setup BEEFY key is not compressed SEC1")
    completed = SETUP / "bridge-ready.json"
    if completed.exists():
        ready = json.loads(completed.read_text())
        require(ready["genesis"] == genesis and ready["bridgeDomain"] == identity["bridgeDomain"] and
                state(api, "Initialized") is True and state(api, "Paused") is False,
                "sealed bridge readiness no longer matches finalized source; HOLD")
        for name in ("funding", "rotation", "unpause"):
            receipt = ready[name]
            if name == "funding" and receipt.get("kind") == "prefunded":
                require(receipt["account"] == Keypair.create_from_uri("//Alice//stash").ss58_address, "wrong prefunded stash")
                verify_prefunded(api, receipt, genesis)
            require(api.get_block_hash(receipt["block"]).lower() == receipt["blockHash"].lower(),
                    f"{name} original receipt became noncanonical; HOLD")
        print(json.dumps({"phase": "ready", "genesis": genesis, "initializedEvent": ready["initializedEvent"],
                          "rotationHash": ready["rotation"]["extrinsicHash"], "unpauseHash": ready["unpause"]["extrinsicHash"]}, sort_keys=True))
        return
    alice = Keypair.create_from_uri("//Alice")
    stash = Keypair.create_from_uri("//Alice//stash")
    keys = api.query("Session", "NextKeys", [stash.ss58_address]).value
    require(isinstance(keys, dict) and len(keys) == 5 and "beefy" in keys and
            (beefy_key.lower() != keys["beefy"].lower() or (SETUP / "rotate-alice.intent.json").exists()),
            "Alice setup BEEFY key is not a fresh session change")
    require(state(api, "Paused") is True or (SETUP / "unpause.intent.json").exists(),
            "source builtin unpaused without original setup transaction; HOLD")
    require(state(api, "Initialized") is False or (SETUP / "rotate-alice.intent.json").exists(),
            "source builtin initialized without original setup transaction; HOLD")
    intent = {"sourceGenesis": genesis, "action": "fund Alice validator stash, set a unique initial BEEFY session key, await BridgeInitialized, sudo-unpause",
              "stash": stash.ss58_address, "setupKeyFile": str(KEY), "setupBeefyPublicKey": beefy_key,
              "reservedWarmupKeyUnchanged": True}
    write_once(SETUP / "initialization-intent.json", intent)
    prefunded_path = SETUP / "prefunded-stash.json"
    initial_height, initial_hash = finalized(api)
    initial_balance = api.query("System", "Account", [stash.ss58_address], block_hash=initial_hash).value["data"]
    if prefunded_path.exists():
        require(not (SETUP / "fund-stash.intent.json").exists(), "conflicting stash funding evidence")
        funding_evidence = json.loads(prefunded_path.read_text())
        require(funding_evidence["account"] == stash.ss58_address, "wrong prefunded stash")
        verify_prefunded(api, funding_evidence, genesis)
    elif (SETUP / "fund-stash.intent.json").exists() or int(initial_balance["free"]) - int(initial_balance["frozen"]) < AMOUNT // 2:
        funding = api.compose_call("Balances", "transfer_keep_alive", {"dest": stash.ss58_address, "value": AMOUNT})
        funding_evidence = submit_once(api, "fund-stash", funding, alice, genesis, ["Balances", "Transfer"])
    else:
        funding_evidence = {"kind": "prefunded", "genesis": genesis, "account": stash.ss58_address,
                            "block": initial_height, "blockHash": initial_hash,
                            "free": str(initial_balance["free"]), "frozen": str(initial_balance["frozen"])}
        write_once(prefunded_path, funding_evidence)
    current_balance = account(api, stash.ss58_address)["data"]
    require(int(current_balance["free"]) - int(current_balance["frozen"]) >= AMOUNT // 2,
            "Alice validator stash lacks transaction fee balance")
    author_has_key = rpc(api, "author_hasKey", [beefy_key, "beef"])
    if not author_has_key:
        inserted = rpc(api, "author_insertKey", ["beef", setup_suri, beefy_key])
        require(inserted is None or isinstance(inserted, str), "author_insertKey response invalid")
    require(rpc(api, "author_hasKey", [beefy_key, "beef"]) is True, "setup BEEFY key absent from Alice keystore")
    keys["beefy"] = beefy_key
    rotation = api.compose_call("Session", "set_keys", {"keys": keys, "proof": "0x"})
    receipt = submit_once(api, "rotate-alice", rotation, stash, genesis)
    if normal and args.stage == "prepare-keys":
        print(json.dumps({"phase": "keys-finalized-awaiting-real-session", "rotation": receipt, "beefyActivation": "NOT AUTHORIZED BY THIS STAGE"}))
        return
    new_keys = api.query("Session", "NextKeys", [stash.ss58_address]).value
    require(new_keys["beefy"].lower() == beefy_key.lower(), "finalized session key did not update")
    start = receipt["block"] + 1
    deadline = time.monotonic() + (1 if normal else 900)
    initialized_block = None
    while time.monotonic() < deadline:
        head, block_hash = finalized(api)
        for number in range(start, head + 1):
            block = api.get_block_hash(number)
            if any((module, name) == ("GearEthBridge", "BridgeInitialized")
                   for event in api.get_events(block)
                   for index, module, name in [event_info(event)]):
                require(state(api, "Initialized", block) is True, "BridgeInitialized event did not initialize storage")
                initialized_block = {"block": number, "hash": block}
                break
        if initialized_block:
            break
        start = head + 1
        time.sleep(3)
    require(initialized_block is not None, "genuine session change did not initialize source bridge; HOLD")
    write_once(SETUP / "bridge-initialized.json", initialized_block)
    if normal:
        head, pin = finalized(api)
        activation = rpc(api, "state_call", ["BeefyApi_beefy_genesis", "0x", pin])
        if args.stage == "activate-beefy":
            import runpy
            authenticate = runpy.run_path(str(Path(__file__).resolve().parents[1] / 'setup-services.py'))['normal_activation_authorities']
            api, launch = profiled_source_pin(identity)
            head, pin = launch['readiness']['commonFinalized']['height'], launch['readiness']['commonFinalized']['hash']
            witness = SubstrateInterface(url=CONFIG['source']['bobRpc'], ss58_format=42)
            snapshots = []
            for node in (api, witness):
                calls = [rpc(node, 'state_call', [method, '0x', pin]) for method in ('BeefyApi_beefy_genesis', 'BeefyApi_validator_set', 'BeefyMmrApi_authority_set_proof', 'BeefyMmrApi_next_authority_set_proof')]
                next_keys = rpc(node, 'state_getStorage', [node.create_storage_key('Beefy', 'NextAuthorities').to_hex(), pin])
                authenticate(calls[1], next_keys, calls[2], calls[3])
                snapshots.append((calls, next_keys))
            require(snapshots[0] == snapshots[1], 'Common finalized current/next authority evidence differs across independent witnesses; HOLD')
            activation = snapshots[0][0][0]
            require(activation == '0x00' or (SETUP / 'activate-beefy.intent.json').exists(), 'BEEFY already activated without original intent; HOLD')
            inner = api.compose_call("Beefy", "set_new_genesis", {"delay_in_blocks": args.activation_delay_blocks})
            call = api.compose_call("Sudo", "sudo", {"call": inner.value})
            activated = submit_once(api, "activate-beefy", call, alice, genesis)
            write_once(SETUP / "activation-approved.json", {"runtimeProfile": CONFIG["runtimeProfile"], "domain": identity["bridgeDomain"], "receipt": activated, "delayBlocks": args.activation_delay_blocks})
            print("Root BEEFY activation finalized; separate unpause still required")
            return
        require((SETUP / "activation-approved.json").exists() and activation.startswith("0x01")
                and len(activation) == 12 and int.from_bytes(bytes.fromhex(activation[4:]), "little") <= head,
                "Original governed activation has not become active; HOLD")
        count = bytes.fromhex(rpc(api, "state_call", ["MmrApi_mmr_leaf_count", "0x", pin])[2:])
        require(len(count) == 9 and count[0] == 0 and int.from_bytes(count[1:], "little") > 0, "Active BEEFY lacks actual MMR insertion; HOLD")
    require(state(api, "Paused") is True or (SETUP / "unpause.intent.json").exists(),
            "source unpaused without original setup transaction; HOLD")
    inner = api.compose_call("GearEthBridge", "unpause", {})
    sudo = api.compose_call("Sudo", "sudo", {"call": inner.value})
    unpause = submit_once(api, "unpause", sudo, alice, genesis, ["GearEthBridge", "BridgeUnpaused"])
    head, block_hash = finalized(api)
    require(state(api, "Initialized", block_hash) is True and state(api, "Paused", block_hash) is False,
            "bridge not initialized and unpaused at finalized head")
    write_once(SETUP / "bridge-ready.json", {"phase": "ready", "genesis": genesis, "bridgeDomain": identity["bridgeDomain"],
              "initializedEvent": initialized_block, "funding": funding_evidence,
              "rotation": receipt, "unpause": unpause, "finalizedHead": {"height": head, "hash": block_hash}})
    print(json.dumps({"phase": "ready", "genesis": genesis, "initializedEvent": initialized_block, "rotationHash": receipt["extrinsicHash"],
                      "unpauseHash": unpause["extrinsicHash"], "finalizedHeight": head}, sort_keys=True), flush=True)


if __name__ == "__main__":
    main()
