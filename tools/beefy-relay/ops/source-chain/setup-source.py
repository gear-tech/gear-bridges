#!/usr/bin/env python3
"""Supervise the two loopback archive validators; readiness is a separate real CLI check."""
import argparse
import fcntl
import json
import os
from pathlib import Path
import plistlib
import re
import subprocess
import sys
from urllib.parse import urlsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from run_context import RUN, CONFIG, artifact, digest, private_text, require, save

SOURCE = RUN / "source-chain"
GEAR = artifact("gear")


def arguments(node, peers):
    port = CONFIG["source"]["p2pPorts"][node]
    args = [str(GEAR), "--chain", str(SOURCE / "chain.raw.json"), "--base-path", str(SOURCE / node),
            "--name", "beefy-hoodi-" + RUN.name[:8] + "-" + node, "--validator", "--" + node, "--force-authoring",
            "--listen-addr", "/ip4/127.0.0.1/tcp/" + str(port), "--public-addr", "/ip4/127.0.0.1/tcp/" + str(port),
            "--rpc-port", str(urlsplit(CONFIG["source"][node + "Rpc"]).port), "--rpc-methods", "unsafe",
            "--state-pruning", "archive", "--blocks-pruning", "archive", "--enable-offchain-indexing=true",
            "--offchain-worker", "always", "--no-mdns", "--no-telemetry",
            "--prometheus-port", str(CONFIG["source"]["metricsPorts"][node]),
            "--node-key-file", str(SOURCE / node / "network-key")]
    if node == "bob":
        args += ["--bootnodes", "/ip4/127.0.0.1/tcp/" + str(CONFIG["source"]["p2pPorts"]["alice"]) + "/p2p/" + peers["alice"]]
    return args


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("stage", choices=("start", "run", "ready"))
    parser.add_argument("--node", choices=("alice", "bob"))
    args = parser.parse_args()
    spec = json.loads((SOURCE / "spec-evidence.json").read_text())
    require(digest(SOURCE / "chain.raw.json") == spec["rawSpecSha256"], "Raw source genesis changed")
    require(spec.get("runtimeProfile") == CONFIG.get("runtimeProfile"), "Source profile changed after immutable preparation")
    plan_path = SOURCE / "supervisor-plan.json"
    if args.stage == "run":
        require(args.node is not None, "Validator role is required")
        plan = json.loads(plan_path.read_text())
        require(plan["rawSpecSha256"] == spec["rawSpecSha256"] and plan["bundleSha256"] == CONFIG["bundle"]["sha256"], "Validator plan changed")
        private_text(SOURCE / args.node / "network-key")
        command = arguments(args.node, plan["peers"])
        os.execve(command[0], command, {**os.environ, "RUST_LOG": "info"})
    if args.stage == "ready":
        subprocess.run([str(artifact("beefy-relay")), "tokens-source-state", "--raw-spec", str(SOURCE / "chain.raw.json"),
                        "--source-rpc", CONFIG["source"]["aliceRpc"], "--witness-rpc", CONFIG["source"]["bobRpc"],
                        "--output", str(SOURCE / "launch-state.json")], check=True, timeout=120)
        return
    lock = (SOURCE / "supervisor-setup.lock").open("a")
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    peers = {}
    for node in ("alice", "bob"):
        key = SOURCE / node / "network-key"
        if not key.exists():
            require(not plan_path.exists(), "Missing original network key; do not regenerate")
            result = subprocess.run([str(GEAR), "key", "generate-node-key", "--chain", "dev", "--file", str(key)], capture_output=True)
            require(result.returncode == 0, "Network key generation failed")
            with key.open("rb") as stream:
                os.fsync(stream.fileno())
        require(re.fullmatch("[0-9a-fA-F]{64}", private_text(key)), "Invalid private network key")
        result = subprocess.run([str(GEAR), "key", "inspect-node-key", "--file", str(key)], capture_output=True, text=True)
        require(result.returncode == 0 and re.fullmatch("12D3KooW[1-9A-HJ-NP-Za-km-z]+", result.stdout.strip()), "Cannot derive original peer identity")
        peers[node] = result.stdout.strip()
    require(peers["alice"] != peers["bob"], "Validator peer identities must differ")
    plan = {"rawSpecSha256": spec["rawSpecSha256"], "bundleSha256": CONFIG["bundle"]["sha256"], "peers": peers}
    if plan_path.exists():
        require(json.loads(plan_path.read_text()) == plan, "Original validator plan changed")
    else:
        save(plan_path, plan)
    domain = "gui/" + str(os.getuid())
    for node in ("alice", "bob", "awake"):
        label = "org.gear.candidate." + RUN.name[:8] + (".source." + node if node != "awake" else ".awake")
        command = ["/usr/bin/caffeinate", "-dimsu"] if node == "awake" else [sys.executable, str(Path(__file__).resolve()), "run", "--node", node]
        directory = SOURCE / node if node != "awake" else RUN / "supervisors"
        configuration = {"Label": label, "ProgramArguments": command, "EnvironmentVariables": {"BEEFY_RUN": str(RUN), "RUST_LOG": "info"},
                         "WorkingDirectory": str(directory), "StandardOutPath": str(directory / "stdout.log"),
                         "StandardErrorPath": str(directory / "stderr.log"), "KeepAlive": True, "ThrottleInterval": 15, "RunAtLoad": True}
        path = RUN / "supervisors" / (node + ".plist")
        data = plistlib.dumps(configuration, sort_keys=False)
        if path.exists():
            require(path.read_bytes() == data, "Supervisor definition changed; reconcile before replacement")
        else:
            with path.open("xb") as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
        target = domain + "/" + label
        if subprocess.run(["/bin/launchctl", "print", target], capture_output=True).returncode != 0:
            subprocess.run(["/bin/launchctl", "bootstrap", domain, str(path)], check=True, capture_output=True)
        subprocess.run(["/bin/launchctl", "print", target], check=True, capture_output=True)
    print("Supervisors installed. Run setup-source.py ready after both validators finalize; process launch is not readiness.")


if __name__ == "__main__":
    main()
