"""Consistency-guard regression only: no RPC, signatures or mocked valid proofs."""

from copy import deepcopy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location("migration_audit", Path(__file__).with_name("zk-migration-audit.py"))
audit = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(audit)


def state():
    b = {name: "0x" + "11" * 32 for name in audit.BINDINGS}
    for name in ("queue", "queueImplementation", "oldVerifier", "manager", "managerQueue", "queueAdmin",
                 "queuePauser", "managerAdmin", "managerPauser", "candidateQueue", "releaseImplementation"):
        b[name] = "0x" + "11" * 20
    b["candidateVerifier"] = "0x" + "22" * 20
    b.update(elChainId=1, candidateDestinationChainId=1, elFinalizedNumber=200, gearFinalizedNumber=200,
             ethereumDiscoveryStartBlock=10, candidateLive=True, candidateMmrStartBlock=1,
             candidateLatestBeefyBlock=180, queuePaused=True, managerPaused=True, sourcePaused=True,
             vftManagers=["0x" + "11" * 32])
    cut = dict(assetFreezeBlock=100, legacyRootCeiling=150, newRootMinimum=151,
               nextNonceAtAssetFreeze=7, executionStopBlock=100)
    preserve = {name: b[name] for name in ("queue", "manager", "gearGenesis", "vftManagers")}
    preserve["replayContinuitySha256"] = "a" * 64
    plan = dict(preserve=preserve, cutover=cut, artifacts={"replayContinuity": {"sha256": "a" * 64}})
    asset = dict(erc20="0x" + "33" * 20, gearToken="0x" + "33" * 32, consumer=b["vftManagers"][0],
                 tokenType=1, supplyType=0, decimalsEth=6, decimalsGear=6, escrowRaw="10", wrappedSupplyRaw="10",
                 gearMinter=b["vftManagers"][0], gearBurner=b["vftManagers"][0])
    b["erc20Tokens"] = [asset["erc20"]]
    claim = dict(direction="ethToGear", key=[1, b["manager"], "0x" + "44" * 32, 0], receiptKey=[60, 0],
                 consumer=asset["consumer"], status="processed", evidence="guard-input-not-a-validity-proof")
    ledger = dict(gearGenesis=b["gearGenesis"], queue=b["queue"], manager=b["manager"], unresolved=[],
                  coverage=dict(completeFromOrigin=True, ethereumStartBlock=10, ethereumThroughBlock=100,
                                sourceThroughBlock=150), claims=[claim])
    row = dict(erc20=asset["erc20"], gearToken=asset["gearToken"], escrowRaw="10", wrappedSupplyRaw="10",
               inboundPendingRaw="0", outboundPendingRaw="0", provenSurplusRaw="0")
    return b, plan, [asset], ledger, {"rows": [row]}


class MigrationGuard(unittest.TestCase):
    def test_original_custody_and_replay_owner_cannot_change(self):
        b, plan, _, _, _ = state()
        audit.check_identity(b, plan)
        for name, change in (
            ("queue", lambda x, p: x.update(candidateQueue="0x" + "22" * 20)),
            ("source", lambda x, p: x.update(candidateSourceGenesis="0x" + "22" * 32)),
            ("chain", lambda x, p: x.update(candidateDestinationChainId=560048)),
            ("custody", lambda x, p: p["preserve"].update(manager="0x" + "22" * 20)),
            ("replay evidence", lambda x, p: p["preserve"].update(replayContinuitySha256="b" * 64)),
            ("unfrozen", lambda x, p: x.update(queuePaused=False)),
            ("root overlap", lambda x, p: p["cutover"].update(newRootMinimum=150)),
        ):
            x, p = deepcopy(b), deepcopy(plan)
            change(x, p)
            with self.subTest(name=name), self.assertRaises(audit.Blocked):
                audit.check_identity(x, p)

    def test_missing_ambiguous_replayed_and_unreconciled_claims_hold(self):
        b, plan, assets, ledger, accounting = state()
        audit.check_ledgers(b, plan["cutover"], assets, ledger, accounting)
        for name, change in (
            ("history gap", lambda l, a: l["coverage"].update(completeFromOrigin=False)),
            ("reserved", lambda l, a: l["claims"][0].update(status="reserved")),
            ("unknown effect", lambda l, a: l.update(unresolved=["original handoff ambiguous"])),
            ("duplicate original", lambda l, a: l["claims"].append(deepcopy(l["claims"][0]))),
            ("wrong custody", lambda l, a: l["claims"][0]["key"].__setitem__(1, "0x" + "22" * 20)),
            ("USDT deficit", lambda l, a: a["rows"][0].update(escrowRaw="14233502454", wrappedSupplyRaw="14233515874")),
            ("unexplained surplus", lambda l, a: a["rows"][0].update(escrowRaw="11", provenSurplusRaw="1")),
            ("made-up pending", lambda l, a: a["rows"][0].update(escrowRaw="11", inboundPendingRaw="1")),
        ):
            l, a = deepcopy(ledger), deepcopy(accounting)
            change(l, a)
            with self.subTest(name=name), self.assertRaises(audit.Blocked):
                audit.check_ledgers(b, plan["cutover"], assets, l, a)
        first, second_asset = deepcopy(assets[0]), deepcopy(assets[0])
        first.update(escrowRaw="10", wrappedSupplyRaw="11")
        second_asset.update(erc20="0x" + "55" * 20, gearToken="0x" + "55" * 32, escrowRaw="11", wrappedSupplyRaw="10")
        net_zero = deepcopy(accounting)
        net_zero["rows"][0].update(escrowRaw="10", wrappedSupplyRaw="11")
        net_zero["rows"].append(dict(net_zero["rows"][0], erc20=second_asset["erc20"], gearToken=second_asset["gearToken"],
                                    escrowRaw="11", wrappedSupplyRaw="10"))
        b["erc20Tokens"] = [first["erc20"], second_asset["erc20"]]
        with self.assertRaises(audit.Blocked):
            audit.check_ledgers(b, plan["cutover"], [first, second_asset], ledger, net_zero)
        with self.assertRaises(audit.Blocked):
            audit.check_ledgers(b, plan["cutover"], assets, ledger, accounting)
        b["erc20Tokens"] = [assets[0]["erc20"]]
        second = deepcopy(ledger["claims"][0])
        second["key"][3] = 1
        second["consumer"] = "0x" + "22" * 32
        b["vftManagers"].append(second["consumer"])
        ledger["claims"].append(second)
        with self.assertRaises(audit.Blocked):
            audit.check_ledgers(b, plan["cutover"], assets, ledger, accounting)

    def test_pre_cutover_messages_cannot_authorize_permissionless_resume(self):
        bindings, _, _, _, _ = state()
        proposals = audit._precutover_proposals(bindings, "0x1234")
        for proposal in proposals:
            command = bytes.fromhex(proposal["args"]["payload"][2:])[0]
            with self.subTest(action=proposal["action"]):
                self.assertNotEqual(command, 2, "ordinary unpause would be executable before postchecks")
                self.assertIn(command, (1, 3))

    def test_compound_snapshot_keeps_nearest_value_provenance(self):
        datum = dict(value={"genesisHash": "0x" + "77" * 32}, status="measured",
                     provenance=["hash-pinned query"], observedAt="2026-10-01", snapshot={"hash": "pin"})
        datum["value"]["missing"] = dict(value={"flag": True}, status="unknown",
                                         provenance=[], observedAt=None, snapshot=None)
        inventory = {"deployments": {"mainnet": {"snapshot": datum}}}
        self.assertEqual(audit.measured(inventory, "/deployments/mainnet/snapshot/value/genesisHash", "mainnet"),
                         datum["value"]["genesisHash"])
        for pointer in ("/deployments/mainnet/snapshot/value/missing/value/flag",
                        "/deployments/mainnet/snapshot/provenance/0",
                        "/deployments/mainnet/snapshot/value/missing/status"):
            with self.subTest(pointer=pointer), self.assertRaises(audit.Blocked):
                audit.measured(inventory, pointer, "mainnet")

    def test_missing_or_advertised_observation_cannot_prepare(self):
        datum = dict(value=1, status="advertised", provenance="public docs", observedAt="2026-10-01", snapshot="pin")
        inventory = {"deployments": {"mainnet": {"chain": datum}, "publicHoodi": {"chain": datum}}}
        for pointer in ("/deployments/mainnet/missing", "/deployments/mainnet/chain", "/deployments/publicHoodi/chain"):
            with self.subTest(pointer=pointer), self.assertRaises(audit.Blocked):
                audit.measured(inventory, pointer, "mainnet")
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "inventory.json"
            path.write_text(json.dumps(inventory))
            result = subprocess.run([sys.executable, str(Path(__file__).with_name("zk-migration-audit.py")),
                                     str(path), "--deployment", "mainnet"], capture_output=True, text=True, check=False)
            output = json.loads(result.stdout)
            self.assertEqual(result.returncode, 2)
            self.assertEqual(output["manifestStatus"], "BLOCKED")
            self.assertFalse(output["executionAuthorized"])


if __name__ == "__main__":
    unittest.main()
