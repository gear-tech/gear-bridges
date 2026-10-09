#!/usr/bin/env python3
"""Seal already-verified artifacts; never build, sign, or deploy from this command."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def require(condition, message):
    if not condition:
        raise SystemExit(message)


def copy_verified(source, target, expected):
    target = Path(target)
    require(expected is not None, "Unqualified artifact: " + str(source))
    shutil.copyfile(source, target)
    require(digest(target) == expected, "Artifact changed while sealing: " + str(source))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--verification", type=Path, required=True)
    parser.add_argument("--ethereum-project", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--runtime-approval", type=Path, help="Independent normal-runtime artifact selection; not an RPC observation")
    parser.add_argument("--runtime-approval-sha256", help="Separately approved SHA256 of the runtime selection document")
    args = parser.parse_args()
    require(__debug__, "Non-optimized Python required")
    verification_bytes = args.verification.read_bytes()
    evidence = json.loads(verification_bytes)
    require(evidence["schemaVersion"] == 1 and evidence["status"] == "VERIFIED", "Verification is not complete")
    import runpy
    runpy.run_path(str(Path(__file__).with_name("run-preflight.py")))["campaign_name"](evidence)
    required = {"cargo-tests", "forge-tests", "full-release-build", "historical-recovery"}
    checks = evidence["checks"]
    require(required <= {check["name"] for check in checks}, "Required qualification checks are missing")
    for check in checks:
        require(check["exitCode"] == 0 and check["command"], "A qualification check failed or lacks its command")
        require(digest(Path(check["logPath"])) == check["logSha256"], "Qualification log changed")
    binaries = evidence["binaries"]
    require(set(binaries) == {"gear", "beefy-relay", "relayer", "checkpoints-tool"}, "Incomplete binary selection")
    profile = evidence.get("runtimeProfile")
    approval_bytes = None
    if profile is not None:
        require('applicationArtifacts' not in evidence, 'Profiled application artifacts are qualified separately after deployment')
        runpy.run_path(str(Path(__file__).with_name("setup-services.py")))["runtime_profile"](profile, binaries["gear"]["sha256"])
        require(args.runtime_approval is not None and args.runtime_approval_sha256 == profile["approvalSha256"].removeprefix("0x"),
                "Profiled runtime requires independently pinned approval bytes; HOLD")
        approval_bytes = args.runtime_approval.read_bytes()
        require(hashlib.sha256(approval_bytes).hexdigest() == args.runtime_approval_sha256, "Runtime approval changed")
        approval = json.loads(approval_bytes)
        require(approval.get("status") == "APPROVED_FOR_TEST_IMPLEMENTATION"
                and approval.get("runtimeProfile") == {key: value for key, value in profile.items() if key != "approvalSha256"},
                "Runtime artifact selection is not independently approved for this exact test profile")
    else:
        require(args.runtime_approval is None and args.runtime_approval_sha256 is None
                and binaries["gear"]["sha256"] == "d25342d65033fdd0d2d04cb4302091aacf7a02c483844a195c71dad2d656b05e",
                "Legacy profile must retain its reviewed executable; normal runtime needs explicit approval")
    for name, entry in binaries.items():
        path = Path(entry["path"])
        require(stat.S_ISREG(path.lstat().st_mode) and os.access(path, os.X_OK), "Binary is not a regular executable: " + name)
        require(digest(path) == entry["sha256"], "Binary changed after qualification: " + name)
    require(evidence["sourceFiles"], "Build source fingerprints missing")
    qualified_sources = {Path(path).resolve(): expected for path, expected in evidence["sourceFiles"].items()}
    for path, expected in qualified_sources.items():
        require(digest(path) == expected, "Source changed after qualification: " + str(path))

    def copy_qualified_source(source, target):
        copy_verified(source, target, qualified_sources.get(Path(source).resolve()))

    project = args.ethereum_project.resolve(strict=True)
    require(digest(project / "out/BeefyTokens.s.sol/BeefyTokens.json") == evidence["solidityArtifactSha256"],
            "Solidity deployment artifact differs from the verified selection")
    infos = list((project / "out/build-info").glob("*.json"))
    require(len(infos) == 1, "Use a clean full Solidity build with exactly one build-info")
    build_bytes = infos[0].read_bytes()
    build = json.loads(build_bytes)
    for relative, source in build["input"]["sources"].items():
        require(not Path(relative).is_absolute() and ".." not in Path(relative).parts, "Unsafe Solidity source path")
        require((project / relative).read_bytes() == source["content"].encode(), "Solidity source changed: " + relative)
    output = args.output.absolute()
    output.mkdir(mode=0o700, parents=False, exist_ok=False)
    (output / "bin").mkdir()
    for name, entry in binaries.items():
        copy_verified(entry["path"], output / "bin" / name, entry["sha256"])
        (output / "bin" / name).chmod(0o500)
    shutil.copytree(Path(__file__).resolve().parent, output / "ops",
                    ignore=shutil.ignore_patterns("__pycache__", "*.pyc"), copy_function=copy_qualified_source)
    solidity = output / "ethereum"
    for name in ("src", "script", "test"):
        (solidity / name).mkdir(parents=True)
    for relative, source in build["input"]["sources"].items():
        target = solidity / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        copy_verified(project / relative, target, hashlib.sha256(source["content"].encode()).hexdigest())
    for name in ("foundry.toml", "foundry.lock", "remappings.txt"):
        if (project / name).exists():
            copy_qualified_source(project / name, solidity / name)
    shutil.copytree(project / "out", solidity / "out", copy_function=copy_qualified_source)
    (solidity / "cache").mkdir()
    copy_qualified_source(project / "cache/solidity-files-cache.json", solidity / "cache/solidity-files-cache.json")
    require(digest(solidity / "out/BeefyTokens.s.sol/BeefyTokens.json") == evidence["solidityArtifactSha256"],
            "Solidity deployment artifact changed while sealing")
    require(digest(solidity / "out/build-info" / infos[0].name) == hashlib.sha256(build_bytes).hexdigest(),
            "Solidity build input changed while sealing")
    copy_verified(args.verification, output / "verification.json", hashlib.sha256(verification_bytes).hexdigest())
    if approval_bytes is not None:
        copy_verified(args.runtime_approval, output / "runtime-approval.json", args.runtime_approval_sha256)
    files = {}
    for path in sorted(output.rglob("*")):
        require(not path.is_symlink(), "Symlink in bundle")
        if path.is_file():
            files[str(path.relative_to(output))] = digest(path)
    manifest = {"schemaVersion": 1, "testOnly": True, "files": files,
                "binaries": {name: "bin/" + name for name in binaries},
                "solidity": {"compilerProjectRoot": str(project), "scriptSha256": files["ethereum/script/BeefyTokens.s.sol"],
                             "artifactSha256": files["ethereum/out/BeefyTokens.s.sol/BeefyTokens.json"]}}
    if profile is not None:
        manifest["runtimeProfile"] = profile
    with (output / "bundle.json").open("x") as stream:
        json.dump(manifest, stream, sort_keys=True, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    for path in output.rglob("*"):
        if path.is_file():
            with path.open("rb") as stream:
                os.fsync(stream.fileno())
            path.chmod(0o500 if path.parent == output / "bin" else 0o400)
    for directory in sorted((p for p in output.rglob("*") if p.is_dir()), key=lambda p: len(p.parts), reverse=True):
        fd = os.open(directory, os.O_RDONLY)
        os.fsync(fd)
        os.close(fd)
        directory.chmod(0o500)
    fd = os.open(output, os.O_RDONLY)
    os.fsync(fd)
    os.close(fd)
    output.chmod(0o500)
    print(json.dumps({"bundle": str(output), "sha256": digest(output / "bundle.json"), "files": len(files)}))


if __name__ == "__main__":
    main()
