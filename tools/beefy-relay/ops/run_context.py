"""Identity and artifact boundary shared by the local Hoodi run commands."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import urllib.request
from urllib.parse import urlsplit

if not __debug__:
    raise SystemExit("Operational safety checks require non-optimized Python")
os.umask(0o077)
OPS = Path(__file__).resolve().parent
try:
    RUN = Path(os.environ["BEEFY_RUN"]).resolve(strict=True)
except (KeyError, OSError):
    raise SystemExit("Set BEEFY_RUN to the prepared run directory") from None


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def load(path):
    return json.loads(Path(path).read_text())


def digest(path):
    with Path(path).open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def private_text(path):
    path = Path(path)
    metadata = path.lstat()
    require(stat.S_ISREG(metadata.st_mode) and stat.S_IMODE(metadata.st_mode) == 0o600,
            "Credential must be a regular mode-0600 file: " + path.name)
    require(0 < metadata.st_size <= 16384, "Credential has invalid size: " + path.name)
    with path.open() as stream:
        actual = os.fstat(stream.fileno())
        require((actual.st_dev, actual.st_ino) == (metadata.st_dev, metadata.st_ino),
                "Credential changed while opening: " + path.name)
        text = stream.read(16385).strip()
    require(text and len(text) <= 16384, "Credential is empty or oversized: " + path.name)
    return text


def save(path, value):
    path = Path(path)
    temporary = path.with_suffix(".tmp")
    fd = os.open(temporary, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "w") as stream:
        json.dump(value, stream, sort_keys=True, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    fd = os.open(path.parent, os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


CONFIG = load(RUN / "run.json")
require(CONFIG["schemaVersion"] == 1 and CONFIG["runId"] == RUN.name
        and CONFIG["testOnly"] is True, "Wrong run identity or unsupported descriptor")
BUNDLE = Path(CONFIG["bundle"]["path"]).resolve(strict=True)
require(digest(BUNDLE / "bundle.json") == CONFIG["bundle"]["sha256"], "Artifact bundle descriptor changed")
MANIFEST = load(BUNDLE / "bundle.json")
require(MANIFEST["schemaVersion"] == 1 and MANIFEST["testOnly"] is True, "Unsupported artifact bundle")
require(MANIFEST["binaries"] == {name: "bin/" + name for name in ("gear", "beefy-relay", "relayer", "checkpoints-tool")},
        "Unexpected executable layout")
NETWORK = CONFIG["network"]
require(NETWORK["chainId"] == 560048 and NETWORK["genesisHash"] ==
        "0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b", "Not the Hoodi test identity")
if MANIFEST.get("runtimeProfile") is not None or CONFIG.get("runtimeProfile") is not None:
    import runpy
    require(CONFIG.get("runtimeProfile") == MANIFEST.get("runtimeProfile"), "Runtime profile changed after preparation")
    runpy.run_path(str(OPS / "setup-services.py"))["runtime_profile"](MANIFEST["runtimeProfile"], MANIFEST["files"]["bin/gear"])
    require(digest(BUNDLE / "runtime-approval.json") == MANIFEST["runtimeProfile"]["approvalSha256"].removeprefix("0x"),
            "Independent runtime approval changed")
for field, scheme in (("executionHttp", "https"), ("executionWss", "wss"), ("beaconHttp", "https")):
    url = urlsplit(NETWORK[field])
    require(url.scheme == scheme and url.hostname and not url.username and not url.password
            and not url.fragment, "Unsafe endpoint: " + field)
for endpoint in (CONFIG["source"]["aliceRpc"], CONFIG["source"]["bobRpc"]):
    url = urlsplit(endpoint)
    require(url.scheme == "ws" and url.hostname == "127.0.0.1" and url.port
            and not url.username and not url.password and not url.query and not url.fragment
            and url.path in ("", "/"), "Source RPC must be a loopback-only endpoint")
require(CONFIG["source"]["aliceRpc"] != CONFIG["source"]["bobRpc"], "Independent source endpoints required")
RPC, BEACON = NETWORK["executionHttp"], NETWORK["beaconHttp"]
SOURCE = Path(CONFIG["funding"]["wallet"])
ADDRESS = CONFIG["funding"]["address"]


def checked_file(relative):
    path = BUNDLE / relative
    require(not Path(relative).is_absolute() and ".." not in Path(relative).parts,
            "Unsafe artifact path")
    require(path.resolve().is_relative_to(BUNDLE) and stat.S_ISREG(path.lstat().st_mode),
            "Artifact must be a regular contained file")
    require(digest(path) == MANIFEST["files"][relative], "Artifact changed: " + relative)
    return path


def artifact(name):
    return checked_file(MANIFEST["binaries"][name])


# Every entry point verifies its own tooling and all executable identities before secret loading.
for relative, expected in MANIFEST["files"].items():
    if relative.startswith("ops/"):
        checked_file(relative)
for name in ("gear", "beefy-relay", "relayer", "checkpoints-tool"):
    artifact(name)
require(OPS == BUNDLE / "ops", "Execute the sealed bundle's operational tooling")
BIN = artifact("beefy-relay").parent


def request(url, body=None):
    request_ = urllib.request.Request(url, None if body is None else json.dumps(body).encode(),
            {"Content-Type": "application/json", "User-Agent": "Mozilla/5.0 (beefy-hoodi-ops)"})
    with urllib.request.urlopen(request_, timeout=45) as response:
        return json.load(response)


def rpc(method, params):
    result = request(RPC, {"jsonrpc": "2.0", "id": 1, "method": method, "params": params})
    if "error" in result:
        message = re.sub(r"0x[0-9a-fA-F]{64,}", "[REDACTED]", str(result["error"]))
        raise RuntimeError(method + ": " + message)
    require(result.get("id") == 1 and "result" in result, "Invalid RPC envelope: " + method)
    return result["result"]


def wallet(path):
    from eth_account import Account
    value = json.loads(private_text(path))
    require(Account.from_key(value["private_key"]).address.lower() == value["address"].lower(),
            "Wallet key/address mismatch: " + Path(path).name)
    return value
