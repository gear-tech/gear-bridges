#!/usr/bin/env python3
"""Mine fixture transactions on an owned loopback Osaka Anvil; retain receipts/calldata."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = "script/BeefyFullTransactionBenchmark.s.sol:BeefyFullTransactionBenchmark"
SENDER = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, default=ROOT / "out/fulltx-benchmark")
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    for binary in ("forge", "anvil", "cast"):
        if not shutil.which(binary):
            parser.error(f"missing installed tool: {binary}")
    # No endpoint/key flags: this command cannot broadcast to a public RPC or
    # mutate the many pre-existing test Anvils owned by other invocations.
    with (output / "anvil.log").open("w") as log:
        node = subprocess.Popen([
            "anvil", "--host", "127.0.0.1", "--port", "0", "--chain-id", "31337",
            "--hardfork", "osaka", "--timestamp", "1800000010",
        ], stdout=log, stderr=subprocess.STDOUT)
    try:
        deadline = time.monotonic() + 30
        while True:
            text = (output / "anvil.log").read_text()
            match = re.search(r"Listening on (127\.0\.0\.1:\d+)", text)
            if match:
                break
            if node.poll() is not None or time.monotonic() >= deadline:
                raise RuntimeError(f"Anvil did not start; see {output / 'anvil.log'}")
            time.sleep(0.1)
        rpc_url = "http://" + match[1]

        def rpc(method, params=None):
            body = json.dumps({"jsonrpc": "2.0", "id": 1, "method": method, "params": params or []}).encode()
            request = urllib.request.Request(rpc_url, body, {"Content-Type": "application/json"})
            with urllib.request.urlopen(request, timeout=30) as response:
                result = json.load(response)
            if "error" in result:
                raise RuntimeError(f"{method}: {result['error']}")
            return result["result"]

        assert int(rpc("eth_chainId"), 16) == 31337
        assert "anvil" in rpc("web3_clientVersion").lower()
        # OpenZeppelin requires project-relative artifacts. Keep them inside the
        # existing read allowlist, isolated by the selected evidence directory.
        build_id = hashlib.sha256(str(output).encode()).hexdigest()
        env = dict(os.environ, FOUNDRY_BROADCAST=str(output / "broadcast"),
                   FOUNDRY_OUT=f"out/fulltx-{build_id}",
                   FOUNDRY_CACHE_PATH=f"cache/fulltx-{build_id}",
                   PRIVATE_KEY="0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
                   BEEFY_BENCHMARK_OWNED_RUNNER="true")

        proxy_functions = {}
        for signature in ("submitMerkleRoot(uint256,bytes32,bytes)",
                          "requestBridging(address,uint256,bytes32)",
                          "processMessage(uint256,uint256,uint256,(uint256,bytes32,address,bytes),bytes32[])"):
            selector = subprocess.check_output(["cast", "sig", signature], text=True).strip()
            proxy_functions[selector] = signature.split("(")[0]

        def function_name(record):
            return (record.get("function") or "").split("(")[0] or proxy_functions.get(
                record["transaction"].get("input", "")[:10], "")

        def broadcast(name, signature=None, parameters=()):
            # Bound synthetic fixture construction separately from mined transactions;
            # Anvil keeps its normal block limit and receipts provide actual gas usage.
            command = ["forge", "script", "--offline", SCRIPT, "--rpc-url", rpc_url,
                       "--gas-limit", "10000000000",
                       "--broadcast", "--slow", "--unlocked", "--sender", SENDER]
            if signature:
                command += ["--sig", signature, *parameters]
            else:
                # ponytail: clean bridge build; reuse mixed build-info when upgrade validation supports it.
                command += ["--force"]
            with (output / f"{name}.log").open("w") as log:
                subprocess.run(command, cwd=ROOT, env=env, stdout=log, stderr=subprocess.STDOUT, check=True)
            function = signature.split("(")[0] if signature else name
            path = output / "broadcast/BeefyFullTransactionBenchmark.s.sol/31337" / f"{function}-latest.json"
            return json.loads(path.read_text())["transactions"]

        transactions = []
        for index, count in ((10, 59), (11, 150), (12, 256)):
            transactions += broadcast(f"runCase{count}", "runCase(uint256)", (str(index),))
        transactions += broadcast("run")

        def target(function):
            matches = [t["transaction"]["to"] for t in transactions if function_name(t) == function]
            assert len(set(matches)) == 1, (function, matches)
            return matches[0]

        queue = target("submitMerkleRoot")
        manager = target("requestBridging")
        token = target("mint")

        def call(address, signature, *parameters):
            return subprocess.check_output(["cast", "call", address, signature, *parameters,
                                            "--rpc-url", rpc_url], cwd=ROOT, text=True).strip().split()[0]

        delay = int(call(queue, "PROCESS_USER_MESSAGE_DELAY()(uint256)"))
        rpc("evm_increaseTime", [delay])
        rpc("evm_mine")
        transactions += broadcast("executeMessage", "executeMessage(address,address,address)", (queue, manager, token))
        assert call(queue, "isProcessed(uint256)(bool)", "702") == "true"
        assert int(call(token, "balanceOf(address)(uint256)", manager)) == 20
        assert int(call(token, "balanceOf(address)(uint256)", SENDER)) == 10
        rows = []
        update_index = 0
        all_receipts = []
        for record in transactions:
            tx = rpc("eth_getTransactionByHash", [record["hash"]])
            receipt = rpc("eth_getTransactionReceipt", [record["hash"]])
            assert tx and receipt and int(receipt["status"], 16) == 1
            block = rpc("eth_getBlockByHash", [receipt["blockHash"], False])
            assert block and record["hash"] in block["transactions"]
            assert rpc("eth_getBlockByNumber", [receipt["blockNumber"], False])["hash"] == block["hash"]
            all_receipts.append(receipt)
            function = function_name(record)
            if function == "submitFiatShamir":
                if update_index < 12:
                    group, step = divmod(update_index, 4)
                    validators = (59, 150, 256)[(group + (step == 3)) % 3]
                    label = ("fixture-first", "ordinary", "handover", "handover-rotated")[step]
                else:
                    assert update_index == 12
                    validators, label = 3, "queue-anchor"
                update_index += 1
            elif function in ("submitMerkleRoot", "processMessage"):
                validators, label = None, function
            else:
                continue
            data = bytes.fromhex(tx["input"][2:])
            selected = None
            if function == "submitFiatShamir":
                proof_offset = int.from_bytes(data[68:100], "big") + 4
                selected = int.from_bytes(data[proof_offset:proof_offset + 32], "big")
            zero = data.count(0)
            nonzero = len(data) - zero
            tokens = zero + 4 * nonzero
            intrinsic = 21_000 + 4 * tokens
            floor = 21_000 + 10 * tokens  # EIP-7623, non-creation tx, no access list.
            gas = int(receipt["gasUsed"], 16)
            assert not tx.get("accessList") and tx["to"] and gas >= max(intrinsic, floor)
            rows.append({"label": label, "validators": validators, "selectedSignatures": selected, "gasUsed": gas,
                         "calldataBytes": len(data), "zeroBytes": zero, "nonzeroBytes": nonzero,
                         "intrinsicGas": intrinsic, "eip7623FloorGas": floor, "floorBinding": gas == floor,
                         "transaction": tx, "receipt": receipt, "block": block})
        assert update_index == 13 and len(rows) == 15
        evidence = {
            "schemaVersion": 1, "chainId": 31337, "hardfork": "osaka",
            "clientVersion": rpc("web3_clientVersion"),
            "forgeVersion": subprocess.check_output(["forge", "--version"], text=True).strip(),
            "fixtureSha256": hashlib.sha256((ROOT / "test/fixtures/beefy-interop.json").read_bytes()).hexdigest(),
            "sourceSha256": {str(path): hashlib.sha256((ROOT / path).read_bytes()).hexdigest() for path in (
                Path("src/beefy/BeefyClient.sol"), Path("src/MessageQueue.sol"), Path("src/ERC20Manager.sol"),
                Path("script/BeefyFullTransactionBenchmark.s.sol"), Path("script/benchmark-full-transactions.py"))},
            "compilerSettings": {"solc": "0.8.37", "viaIR": True, "optimizer": True, "evmVersion": "osaka"},
            "postState": {"queue": queue, "manager": manager, "token": token,
                          "message702Processed": True, "managerBalance": 20, "recipientBalance": 10},
            "measurements": rows, "allReceipts": all_receipts,
        }
        path = output / "receipts.json"
        path.write_text(json.dumps(evidence, indent=2) + "\n")
        print("validators signatures label gasUsed calldataBytes intrinsicGas eip7623FloorGas floorBinding")
        for row in rows:
            print(*(row[k] for k in ("validators", "selectedSignatures", "label", "gasUsed", "calldataBytes", "intrinsicGas", "eip7623FloorGas", "floorBinding")))
        print(f"Evidence: {path}")
    finally:
        node.terminate()
        try:
            node.wait(timeout=10)
        except subprocess.TimeoutExpired:
            node.kill()
            node.wait()


if __name__ == "__main__":
    main()
