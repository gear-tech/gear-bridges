#!/usr/bin/env python3
"""Run: python tools/beefy-relay/ops/test_ops.py. No network calls or signing."""
import hashlib
import json
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import tempfile
import sys
from unittest.mock import patch


def rejected(action, message=None):
    try:
        action()
    except RuntimeError as error:
        if message is not None:
            assert message in str(error), str(error)
        return
    raise AssertionError("Unsafe operational input was accepted")

def check_bootstrap_source_inventory():
    validate = runpy.run_path(str(Path(__file__).with_name('bootstrap-queue.py')))['bootstrap_assets_are_unbridged']
    ethereum = {symbol: {'erc20Supply': '100', 'evmUser': '100', 'evmManagerEscrow': '0', 'gearUser': '0', 'vftSupply': '0'} for symbol in ('USDC', 'USDT', 'WBTC', 'WETH')}
    assert validate(ethereum, None)
    inventory = {'status': 'ready', 'assets': {'GOT': {'amountRaw': '100'}, 'WTVARA': {'amountRaw': '2000000000000'}}}
    assets = {**ethereum, **{symbol: {'erc20Supply': '0', 'evmUser': '0', 'evmManagerEscrow': '0', 'gearManagerEscrow': '0', 'gearUser': item['amountRaw'], 'vftSupply': item['amountRaw']} for symbol, item in inventory['assets'].items()}}
    assert validate(assets, inventory)
    assert not validate(assets, None)
    assert not validate(ethereum, inventory)
    assert not validate(assets, {**inventory, 'status': 'pending'})
    assert not validate({**assets, 'UNKNOWN': ethereum['USDC']}, inventory)
    for symbol, fields in [('GOT', ('erc20Supply', 'evmUser', 'evmManagerEscrow', 'gearManagerEscrow', 'gearUser', 'vftSupply')), ('USDC', ('evmManagerEscrow', 'gearUser', 'vftSupply'))]:
        for field in fields:
            changed = {**assets, symbol: {**assets[symbol], field: str(int(assets[symbol][field]) + 1)}}
            assert not validate(changed, inventory), (symbol, field)

def check_normal_activation_authorities():
    from Crypto.Hash import keccak
    from eth_keys import keys
    authenticate = runpy.run_path(str(Path(__file__).with_name('setup-services.py')))['normal_activation_authorities']
    public = [keys.PrivateKey(number.to_bytes(32, 'big')).public_key for number in (1, 2, 3)]
    def frames(values):
        def vector(values):
            return bytes([len(values) << 2]) + b''.join(value.to_compressed_bytes() for value in values)
        def root(values):
            layer = [keccak.new(digest_bits=256, data=value.to_canonical_address()).digest() for value in values]
            while len(layer) > 1:
                layer = [keccak.new(digest_bits=256, data=layer[index]+layer[index+1]).digest() if index+1 < len(layer) else layer[index] for index in range(0, len(layer), 2)]
            return layer[0]
        def proof(identifier, values):
            return identifier.to_bytes(8, 'little') + len(values).to_bytes(4, 'little') + root(values)
        return tuple('0x'+value.hex() for value in (b'\x01'+vector(values)+(7).to_bytes(8, 'little'), vector(values[::-1]), proof(7, values), proof(8, values[::-1])))
    for count in (1, 2, 3):
        valid = frames(public[:count])
        assert authenticate(*valid) == {'currentId': 7, 'currentCount': count, 'nextId': 8, 'nextCount': count}
    valid = frames(public[:2])
    for position, value in ((0, '0x00'), (0, valid[0]+'00'), (1, valid[1]+'00'), (2, valid[2]+'00'), (3, '0x'+('09'+'00'*7)+valid[3][18:]), (2, valid[2][:-2]+'00'), (0, '0x01'+((257 << 2)|1).to_bytes(2, 'little').hex()), (0, '0x010900'+valid[0][6:]), (0, '0x0104'+'02'+'ff'*32+'07'+'00'*7)):
        changed = list(valid); changed[position] = value
        rejected(lambda: authenticate(*changed))
    rejected(lambda: authenticate(*frames([public[0], public[0]])))


def check_profiled_application_qualification():
    admission = runpy.run_path(str(Path(__file__).with_name('run-preflight.py')))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary).resolve()
        bundle, artifacts = root / 'sealed', root / 'app-messages/qualified'
        bundle.mkdir(); artifacts.mkdir(parents=True); (root / 'qualification').mkdir()
        campaign, profile = 'hoodi-fast-runtime-unit', {'name': 'fast-runtime-hoodi'}
        files = {}
        for relative in ('bin/node', 'lib/demo/example/eth-to-vara.js', 'lib/demo/example/vara-to-eth.js'):
            path = artifacts / relative
            path.parent.mkdir(parents=True, exist_ok=True); path.write_text('owned test artifact: ' + relative)
            path.chmod(0o500 if relative == 'bin/node' else 0o400)
            files[relative] = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest = artifacts / 'manifest.json'
        manifest.write_text(json.dumps({'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'campaignName': campaign, 'files': files}))
        manifest.chmod(0o400)
        for path in artifacts.rglob('*'):
            if path.is_dir(): path.chmod(0o500)
        artifacts.chmod(0o500)
        verification = bundle / 'verification.json'
        verification.write_text(json.dumps({'campaignName': campaign, 'runtimeProfile': profile}))
        verification_sha = hashlib.sha256(verification.read_bytes()).hexdigest()
        (bundle / 'bundle.json').write_text(json.dumps({'files': {'verification.json': verification_sha}}))
        checks = []
        for index, command in enumerate(sorted(admission['APPLICATION_CHECK_COMMANDS'])):
            log = root / 'qualification' / (str(index) + '.log')
            log.write_text('unit evidence fixture for ' + command); log.chmod(0o400)
            checks.append({'command': command, 'exitCode': 0, 'logPath': str(log), 'logSha256': hashlib.sha256(log.read_bytes()).hexdigest()})
        qualified = {'schemaVersion': 1, 'testOnly': True, 'status': 'VERIFIED', 'runId': root.name, 'campaignName': campaign, 'runtimeProfile': profile,
            'componentBundleSha256': hashlib.sha256((bundle / 'bundle.json').read_bytes()).hexdigest(), 'componentVerificationSha256': verification_sha,
            'checks': checks, 'applicationArtifacts': {'path': str(artifacts), 'manifestSha256': hashlib.sha256(manifest.read_bytes()).hexdigest()}}
        record = root / 'qualification' / (campaign + '-applications.json')
        record.write_text(json.dumps(qualified)); record.chmod(0o400)
        original = record.read_bytes()
        args = (root, campaign, 'eth-to-vara', bundle, verification_sha)
        assert admission['app_artifact'](*args) == (artifacts / 'bin/node', artifacts / 'lib/demo/example/eth-to-vara.js')
        for change in ({'runId': 'other'}, {'componentBundleSha256': '00' * 32}, {'componentVerificationSha256': '00' * 32},
                       {'runtimeProfile': {'name': 'normal-runtime-hoodi'}}, {'status': 'FAILED'}, {'checks': checks[:-1]},
                       {'checks': [{**checks[0], 'exitCode': 1}, *checks[1:]]}):
            rejected(lambda: admission['app_artifact'](*args, qualification={**qualified, **change}))
        record.chmod(0o600)
        rejected(lambda: admission['app_artifact'](*args), 'Immutable post-deployment')
        record.chmod(0o400)
        log = Path(checks[0]['logPath']); log.chmod(0o600); log.write_text('changed original check'); log.chmod(0o400)
        rejected(lambda: admission['app_artifact'](*args), 'Immutable application check evidence changed')
        assert record.read_bytes() == original



def check_normal_runtime_admission():
    services = runpy.run_path(str(Path(__file__).with_name('setup-services.py')))
    admission = runpy.run_path(str(Path(__file__).with_name('run-preflight.py')))
    profile = {'name': 'normal-runtime-hoodi', 'runtimeCommit': '19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c',
               'runtimePullRequest': 5642, 'slotDurationMs': 3000, 'epochDurationBlocks': 2400,
               'warmupDurationMs': 18000000, 'requiredAuthorityHandovers': 2, 'tokenBatchDurationMs': 3600000,
               'applicationAttemptDurationMs': 2640000, 'testOnly': True, 'executionAuthorized': False, 'releaseQualified': False,
               'runtimeCiStatus': 'unresolved', **{name: '11' * 32 for name in
                    ('gearBinarySha256', 'runtimeCodeSha256', 'runtimeCodeBlake2b256', 'runtimeCodeKeccak256', 'approvalSha256')}}
    check_profile = services['runtime_profile']
    assert check_profile(profile, '11' * 32) == profile
    for name, bad in (('epochDurationBlocks', 64), ('warmupDurationMs', 3600000), ('requiredAuthorityHandovers', 1),
                      ('tokenBatchDurationMs', 18000000), ('applicationAttemptDurationMs', 18000000),
                      ('executionAuthorized', True), ('releaseQualified', True), ('runtimeCommit', 'fast-runtime'), ('runtimeCommit', 'f961bed815dd4ab0802703620605ea3b3659ac60'),
                      ('gearBinarySha256', '00' * 32), ('approvalSha256', '00' * 32)):
        rejected(lambda: check_profile({**profile, name: bad}, '11' * 32))
    rejected(lambda: check_profile(profile, '22' * 32))
    fast = {**profile, 'name': 'fast-runtime-hoodi', 'epochDurationBlocks': 64,
            'warmupDurationMs': 3600000, 'functionalOnly': True, 'cadencePatchSha256': '33' * 32}
    assert check_profile(fast, '11' * 32) == fast
    for name, bad in (('name', 'normal-runtime-hoodi'), ('epochDurationBlocks', 2400),
                      ('warmupDurationMs', 18000000), ('functionalOnly', False),
                      ('cadencePatchSha256', None), ('cadencePatchSha256', '00' * 32),
                      ('requiredAuthorityHandovers', 1), ('executionAuthorized', True)):
        rejected(lambda: check_profile({**fast, name: bad}, '11' * 32))
    rejected(lambda: check_profile({**profile, 'functionalOnly': True}, '11' * 32))
    rejected(lambda: check_profile({**profile, 'cadencePatchSha256': '33' * 32}, '11' * 32))
    assert admission['campaign_name']({'campaignName': 'hoodi-fast-runtime-test', 'runtimeProfile': fast}) == 'hoodi-fast-runtime-test'
    rejected(lambda: admission['campaign_name']({'campaignName': 'hoodi-normal-runtime-test', 'runtimeProfile': fast}))
    rejected(lambda: admission['campaign_name']({'campaignName': 'hoodi-fast-runtime-test', 'runtimeProfile': profile}))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary) / 'fresh-run'; root.mkdir(); (root / 'supervisors').mkdir()
        bundle = Path(temporary) / 'bundle'; (bundle / 'ops').mkdir(parents=True)
        files = {}
        for name in ('run-preflight.py', 'setup-services.py', 'warmup-supervisor-observer.py'):
            shutil.copyfile(Path(__file__).with_name(name), bundle / 'ops' / name)
            files['ops/' + name] = hashlib.sha256((bundle / 'ops' / name).read_bytes()).hexdigest()
        qualification = {'campaignName': 'hoodi-normal-runtime-test', 'runtimeProfile': profile}
        (bundle / 'verification.json').write_text(json.dumps(qualification))
        files['verification.json'] = hashlib.sha256((bundle / 'verification.json').read_bytes()).hexdigest(); files['bin/gear'] = '11' * 32
        manifest = {'files': files, 'runtimeProfile': profile}; config = {'bundle': {'sha256': '22' * 32}, 'runtimeProfile': profile}
        launch = {'identity': {'runtimeProfile': profile}}
        for relative in ('source-chain/launch-state.json', 'source-chain/chain.raw.json', 'source-chain/identity.json',
                         'deployment.json', 'token-stack/token-stack.json', 'qualification/components.json'):
            path = root / relative; path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(json.dumps(launch if relative.endswith('launch-state.json') else {}))
        services['prepare_runtime_campaign'](root, config, bundle, manifest, launch)
        original = (root / 'supervisors/normal-campaign-admission.json').read_bytes()
        services['prepare_runtime_campaign'](root, config, bundle, manifest, launch)
        assert (root / 'supervisors/normal-campaign-admission.json').read_bytes() == original
        admission['completed_transition'](root, config, bundle, manifest)
        assert not (root / 'supervisors/hoodi-milestone-1-observer-binding.json').exists()
        import plistlib
        for plist in (root / 'supervisors').glob('*.plist'):
            definition = plistlib.loads(plist.read_bytes())
            assert definition['KeepAlive'] is False and definition['RunAtLoad'] is True
        (root / 'deployment.json').write_text('{"changed":true}')
        rejected(lambda: admission['completed_transition'](root, config, bundle, manifest))
        qualification['campaignName'] = 'hoodi-milestone-1'
        (bundle / 'verification.json').write_text(json.dumps(qualification))
        rejected(lambda: admission['qualified_campaign'](bundle, hashlib.sha256((bundle / 'verification.json').read_bytes()).hexdigest()))


def check_checkpoint_trust_inputs():
    script = Path(__file__).with_name('prepare-run.py')
    hoodi = '0x212f13fc4df078b6cb7db228f1c8307566dcecf900867401a92023d7ba99cb5f'
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        for anchor, genesis in (('0x' + '00' * 32, hoodi), ('0x' + '11' * 32, '0x' + '22' * 32)):
            result = subprocess.run([sys.executable, str(script), '--runtime-profile', 'legacy-hoodi', '--runs-dir', str(root),
                '--funding-wallet', str(root / 'wallet'), '--funding-address', '0x' + '33' * 20,
                '--trusted-bootstrap-root', anchor, '--trusted-genesis-validators-root', genesis],
                capture_output=True, text=True, timeout=10)
            assert result.returncode == 2, result.stderr
            assert not any(root.iterdir()), 'Untrusted checkpoint preparation created run state'


def solidity_snapshot(bundle, project):
    solidity = bundle / "ethereum"
    targets = {
        "BeefyTokens": "script/BeefyTokens.s.sol",
        "BeefyClient": "src/beefy/BeefyClient.sol",
        "VaraQueueRootVerifier": "src/VaraQueueRootVerifier.sol",
        "MessageQueue": "src/MessageQueue.sol",
        "ERC20Manager": "src/ERC20Manager.sol",
        "WrappedVara": "src/erc20/WrappedVara.sol",
        "RecoveryController": "src/RecoveryController.sol",
        "ERC1967Proxy": "dependencies/@openzeppelin-contracts-5.7.0/proxy/ERC1967/ERC1967Proxy.sol",
    }
    from eth_utils import keccak
    sources = {source: {"content": "contract " + name + " {}\n"} for name, source in targets.items()}
    metadata_sources = {name: {"keccak256": "0x" + keccak(text=source["content"]).hex()}
                        for name, source in sources.items()}
    contracts = {}
    for index, (name, source) in enumerate(targets.items()):
        path = solidity / source
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(sources[source]["content"])
        abi = ([{"name": "run", "type": "function", "inputs": [],
                 "outputs": [{"type": "address"}] * 4}] if name == "BeefyTokens" else [])
        evm = {"bytecode": {"object": f"60{index:02x}6000"}, "deployedBytecode": {"object": f"60{index:02x}"}}
        contracts[source] = {name: {"abi": abi, "evm": evm}}
        artifact = {"abi": abi, "metadata": {"compiler": {"version": "0.8.37+fixture"},
                    "settings": {"compilationTarget": {source: name}}, "sources": metadata_sources},
                    **{key: {"object": "0x" + value["object"]} for key, value in evm.items()}}
        out = solidity / "out" / Path(source).name / (name + ".json")
        out.parent.mkdir(parents=True, exist_ok=True)
        out.write_text(json.dumps(artifact))
    (solidity / "test").mkdir()
    (solidity / "out/build-info").mkdir()
    (solidity / "out/build-info/fixture.json").write_text(json.dumps({"input": {"sources": sources},
                                                                      "output": {"contracts": contracts}}))
    (solidity / "foundry.toml").write_text('[profile.default]\nsrc = "src"\nout = "out"\n')
    (solidity / "remappings.txt").write_text("fixture/=src/\n")
    (solidity / "cache").mkdir()
    (solidity / "cache/solidity-files-cache.json").write_text(json.dumps({"builds": ["fixture"]}))
    shutil.copytree(solidity, project)


def check_deployment_inputs(deploy, project, bundle):
    deploy["compiled_artifact"]()
    wrapped = project / "out/WrappedVara.sol/WrappedVara.json"
    raw = wrapped.read_bytes()
    changes = []
    for field in ("abi", "bytecode", "deployedBytecode"):
        value = json.loads(raw)
        if field == "abi":
            value[field].append({"name": "unexpected", "type": "function", "inputs": [], "outputs": []})
        else:
            value[field]["object"] += "01"
        changes.append(("WrappedVara " + field, wrapped, json.dumps(value).encode()))
    build_path = project / "out/build-info/fixture.json"
    build = json.loads(build_path.read_bytes())
    build["output"]["contracts"]["src/erc20/WrappedVara.sol"]["WrappedVara"]["evm"]["bytecode"]["object"] += "01"
    changes.append(("unsealed build-info", build_path, json.dumps(build).encode()))
    for name in ("foundry.toml", "remappings.txt"):
        path = project / name
        changes += [(name + " changed", path, path.read_bytes() + b"# unqualified settings\n"),
                    (name + " missing", path, None), (name + " symlink", path, bundle / "ethereum" / name)]
    cache = project / "cache/solidity-files-cache.json"
    changes += [("redirected compiler cache", cache, json.dumps({"builds": ["unqualified"]}).encode()),
                ("missing compiler cache", cache, None),
                ("symlinked compiler cache", cache, bundle / "ethereum/cache/solidity-files-cache.json")]
    changes.append(("unqualified foundry.lock", project / "foundry.lock", b"unqualified lock\n"))
    accepted = []
    for label, path, replacement in changes:
        original = path.read_bytes() if path.exists() else None
        try:
            if path.exists():
                path.unlink()
            if isinstance(replacement, Path):
                path.symlink_to(replacement)
            elif replacement is not None:
                path.write_bytes(replacement)
            try:
                deploy["compiled_artifact"]()
            except RuntimeError:
                pass
            else:
                accepted.append(label)
        finally:
            if path.exists() or path.is_symlink():
                path.unlink()
            if original is not None:
                path.write_bytes(original)
    assert not accepted, "Unqualified deployment inputs accepted: " + ", ".join(accepted)
    config = project / "foundry.toml"
    original = config.read_bytes()
    try:
        config.write_bytes(original + b"# unqualified settings\n")
        for options in (("--dry-run",), ("--check",), ()):
            result = subprocess.run([sys.executable, str(bundle / "ops/prepare-token-deployment.py"), *options],
                                    capture_output=True, text=True, timeout=10)
            assert result.returncode != 0
            assert not any((project / name).exists() for name in ("simulation.json", "deployment-intent.json", "deployment.log"))
            print("Isolated entrypoint", options or ("broadcast",), result.stderr.strip())
    finally:
        config.write_bytes(original)


def check_simulation_binding(deploy, project, files):
    main = deploy["main"]
    deployer = "0x" + "11" * 20
    inputs = {"identity": {"deployerAddress": deployer, "nonce": {"rpc": "https://invalid.example"}},
              "stack": {"programs": {"vftManager": {"id": "0x" + "22" * 32}}},
              "gearAdmin": "0x" + "33" * 32, "gearPauser": "0x" + "44" * 32,
              "source": "0x" + "55" * 32, "bridge": "0x" + "66" * 32,
              "roles": {"root": "0x" + "77" * 20}, "inputSha256": {"fixture": "8" * 64},
              "anchor": {"mmrStartBlock": 1, "block": 10, "sourceTimestampMs": 30000,
                         "blockHash": "0x" + "99" * 32,
                         "current": {"id": 0, "keys": ["fixture"], "root": "0x" + "aa" * 32},
                         "next": {"id": 1, "keys": ["fixture"], "root": "0x" + "bb" * 32}},
              "queue": "0x" + "cc" * 20, "genesis": "0x" + "dd" * 32}
    dry_run = project / "broadcast/BeefyTokens.s.sol/560048/dry-run/run-latest.json"
    simulation = project / "simulation.json"
    intent = project / "deployment-intent.json"
    handoffs = []

    def forge(command, *args, **kwargs):
        handoffs.append("--broadcast" in command)
        dry_run.parent.mkdir(parents=True, exist_ok=True)
        dry_run.write_text(json.dumps({"chain": 560048, "receipts": [], "pending": [],
                                       "transactions": [{"hash": None}]}))

    def invoke(*options):
        with patch.object(sys, "argv", ["prepare-token-deployment.py", *options]):
            main()

    # Only external network, credential and Forge handoffs are replaced; admission and persistence are real.
    external = {"source_inputs": lambda: inputs, "key": lambda _: ("0x" + "ee" * 32, deployer),
                "recovery_and_chain": lambda *_: {"wallet": "0x" + "ff" * 20},
                "nonce_zero": lambda *_: None, "rpc": lambda *_: "0x", "run_forge": forge}
    with patch.dict(main.__globals__, external), patch.dict(os.environ, {"ETHERSCAN_API_KEY": "test-only"}):
        invoke("--dry-run")
        saved = json.loads(simulation.read_bytes())
        invoke("--check")
        assert handoffs == [False] and not intent.exists()
        legacy = json.loads(simulation.read_bytes())
        legacy["compiled"].pop("foundryFilesSha256", None)
        legacy["compiled"]["contracts"].pop("WrappedVara", None)
        simulation.write_text(json.dumps(legacy))
        for options in (("--check",), ()):
            rejected(lambda: invoke(*options))
        assert handoffs == [False] and not intent.exists()
        simulation.write_text(json.dumps(saved))
        config = project / "foundry.toml"
        original = config.read_bytes()

        def changed_inputs():
            config.write_bytes(original + b"# changed during readiness\n")
            return inputs

        for options in (("--dry-run",), ("--check",), ()):
            try:
                with patch.dict(main.__globals__, {"source_inputs": changed_inputs}):
                    rejected(lambda: invoke(*options))
                assert handoffs == [False] and not intent.exists()
                assert json.loads(simulation.read_bytes()) == saved
            finally:
                config.write_bytes(original)
        nonces_checked = 0

        def changed_after_intent(*_):
            nonlocal nonces_checked
            nonces_checked += 1
            if nonces_checked == 2:
                config.write_bytes(original + b"# changed after intent\n")

        try:
            with patch.dict(main.__globals__, {"nonce_zero": changed_after_intent}):
                rejected(invoke)
            recorded = json.loads(intent.read_bytes())
            assert recorded["compiled"] == saved["compiled"] and handoffs == [False]
        finally:
            config.write_bytes(original)
        rejected(invoke)
        assert json.loads(intent.read_bytes()) == recorded and handoffs == [False]



def check_milestone_ops():
    import fcntl
    import plistlib
    import types
    import time
    from eth_utils import keccak
    ops = Path(__file__).parent
    admission = runpy.run_path(str(ops / 'run-preflight.py'))
    services = runpy.run_path(str(ops / 'setup-services.py'))
    observer = runpy.run_path(str(ops / 'warmup-supervisor-observer.py'))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary).resolve() / 'run'
        root.mkdir(mode=0o700)
        campaign = root / 'campaigns/hoodi-milestone-1'
        for name in ('../campaign', 'a/b', '', 'a' * 65, '_unsafe'):
            rejected(lambda name=name: admission['campaign_directory'](root, name, 'preflight'))
        (root / 'campaigns').symlink_to(Path(temporary))
        rejected(lambda: admission['campaign_directory'](root, 'hoodi-milestone-1', 'preflight'))
        (root / 'campaigns').unlink()
        campaign.mkdir(mode=0o700, parents=True)
        assert admission['campaign_directory'](root, 'hoodi-milestone-1', 'preflight') == campaign
        stale = campaign / 'observer.log'
        stale.write_text('not an empty native preflight directory')
        rejected(lambda: admission['campaign_directory'](root, 'hoodi-milestone-1', 'preflight'))
        stale.unlink()
        journal = campaign / 'campaign-state.json'
        value = {'schemaVersion': 3, 'runId': 'old-campaign', 'preflight': {'status': 'pending'}}
        journal.write_text(json.dumps(value)); journal.chmod(0o600)
        rejected(lambda: admission['campaign_directory'](root, 'hoodi-milestone-1', 'preflight'))
        value.update(runId='hoodi-milestone-1', preflight={'status': 'failed'})
        journal.write_text(json.dumps(value))
        rejected(lambda: admission['campaign_directory'](root, 'hoodi-milestone-1', 'preflight'))
        # The lock stays owned by the actual child after the admission parent closes it.
        with patch.dict(os.environ, {}, clear=True):
            lock = admission['campaign_lock'](root)
            child = subprocess.Popen([sys.executable, '-c', 'import sys; print("held",flush=True); sys.stdin.readline()',
                                      str(lock.fileno())], pass_fds=(lock.fileno(),), stdin=subprocess.PIPE,
                                     stdout=subprocess.PIPE, text=True)
            try:
                assert child.stdout.readline().strip() == 'held'
                lock.close()
                try:
                    admission['campaign_lock'](root)
                except BlockingIOError:
                    pass
                else:
                    raise AssertionError('Parent exit released the live child campaign lock')
            finally:
                child.communicate('\n', timeout=10)
            with admission['campaign_lock'](root) as inherited:
                with patch.dict(os.environ, {'BEEFY_CAMPAIGN_LOCK_FD': str(inherited.fileno())}):
                    with admission['campaign_lock'](root):
                        pass  # Same open-file ownership is inherited, not reacquired independently.
            with (root / 'bounded-campaign.lock').open('a') as unlocked:
                with patch.dict(os.environ, {'BEEFY_CAMPAIGN_LOCK_FD': str(unlocked.fileno())}):
                    rejected(lambda: admission['campaign_lock'](root))
        # Real CAS resumes one intended replacement and never accepts a third value.
        mutable = root / 'mutable.json'
        mutable.write_bytes(b'old')
        old_hash, new_hash = (hashlib.sha256(blob).hexdigest() for blob in (b'old', b'new'))
        temporary_write = mutable.with_name(mutable.name + '.transition-tmp')
        temporary_write.write_bytes(b'new')
        services['cas_replace'](mutable, old_hash, new_hash, b'new')
        assert mutable.read_bytes() == b'new' and not temporary_write.exists()
        services['cas_replace'](mutable, old_hash, new_hash, b'new')
        mutable.write_bytes(b'third')
        rejected(lambda: services['cas_replace'](mutable, old_hash, new_hash, b'new'))
        assert mutable.read_bytes() == b'third'
        original = {'Label': 'original', 'ProgramArguments': ['/python', '/old/ops/setup-services.py', 'run', 'follower'],
                    'EnvironmentVariables': {'BEEFY_RUN': '/retained'}, 'KeepAlive': True, 'RunAtLoad': True,
                    'WorkingDirectory': '/private', 'StandardOutPath': '/logs/old', 'ThrottleInterval': 15}
        replaced = plistlib.loads(services['replacement_plist'](plistlib.dumps(original), Path('/old'), Path('/new')))
        assert replaced == {**original, 'ProgramArguments': ['/python', '/new/ops/setup-services.py', 'run', 'follower']}
        rejected(lambda: services['replacement_plist'](plistlib.dumps({**original, 'AbandonProcessGroup': True}), Path('/old'), Path('/new')))
        rejected(lambda: services['remaining']({'name': 'source', 'status': 'RUNNING', 'deadlineAtMs': 1}))
        rejected(lambda: services['remaining']({'name': 'source', 'status': 'FAILED', 'deadlineAtMs': int(time.time() * 1000) + 60000}))
        manifest = {'files': {'bin/gear': services['GEAR_SHA'], 'bin/relayer': 'same', 'bin/checkpoints-tool': 'same',
                              'bin/beefy-relay': 'old', 'ethereum/program.abi': 'same'}, 'solidity': {'artifact': 'same'}, 'binaries': {}}
        changed = {**manifest, 'files': {**manifest['files'], 'bin/beefy-relay': 'new'}}
        assert set(services['bundle_diff'](manifest, changed)) == {'bin/beefy-relay'}
        for field in ('bin/relayer', 'bin/gear', 'ethereum/program.abi'):
            rejected(lambda field=field: services['bundle_diff'](manifest, {**changed, 'files': {**changed['files'], field: 'changed'}}))
        # Bootout success/job absence alone is NOT a stop barrier: a native process still owns the lane.
        state = {'actors': {name: {'label': name, 'pid': 7} for name in services['ACTORS']}, 'stopIntents': {},
                 'checkpoint': 'checkpoint', 'originalProcesses': [], 'stopGate': {
                     'name': 'stop', 'status': 'RUNNING', 'deadlineAtMs': int(time.time() * 1000) + 25}}
        table = {7: {'pid': 7, 'ppid': 1, 'pgid': 7, 'started': 'original', 'command': '/native/relayer --storage-path ' + str(root) + '/inbound'}}
        with patch.dict(services['stop_barrier'].__globals__, launch_observation=lambda *args: None, process_table=lambda *args: table):
            rejected(lambda: services['stop_barrier'](root, 'gui/0', state, root / 'stop.json', state['stopGate']))
        saved = json.loads((root / 'stop.json').read_text())
        assert saved['stopGate']['status'] == 'FAILED' and 'wrapperAndNativeExited' not in saved['stopGate']

        # A complete interrupted journal save resumes its exact original deadlines.
        recovery = root / 'transition.json'
        recovery_state = {'runId': 'original', 'status': 'RUNNING', 'phase': 'prepared', 'lastWallMs': 1,
            'stopIntents': {}, 'starts': {}, 'stopGate': {'name': 'stop', 'status': 'RUNNING', 'deadlineAtMs': 40000},
            'sourceGate': {'name': 'source', 'status': 'PENDING', 'absoluteCapAtMs': 160000},
            'progressGate': {'name': 'progress', 'status': 'PENDING', 'deadlineAtMs': 2700000}}
        recovery.write_bytes(services['json_bytes'](recovery_state)); recovery.chmod(0o600)
        pending = {**recovery_state, 'phase': 'fenced', 'lastWallMs': 2,
                   'stopGate': {**recovery_state['stopGate'], 'status': 'PASS'}}
        save_marker = recovery.with_name('transition.json.transition-tmp')
        save_marker.write_bytes(services['json_bytes'](pending)); save_marker.chmod(0o600)
        loaded, save = services['transition_journal'](recovery)
        assert loaded == pending
        services['cas_replace'](recovery, *save)
        assert json.loads(recovery.read_text())['stopGate']['deadlineAtMs'] == 40000 and not save_marker.exists()
        malicious = {**pending, 'stopGate': {**pending['stopGate'], 'deadlineAtMs': 70000}}
        save_marker.write_bytes(services['json_bytes'](malicious)); save_marker.chmod(0o600)
        rejected(lambda: services['transition_journal'](recovery))
        assert save_marker.exists() and json.loads(recovery.read_text()) == pending
        save_marker.unlink()
        # A resumed source gate reauthenticates a fresh observation without replacing its original evidence.
        source_record = root / 'transition-source/transition.json'
        source_record.parent.mkdir(mode=0o700)
        candidate = root / 'candidate'
        (candidate / 'bin').mkdir(mode=0o700, parents=True)
        (candidate / 'bin/beefy-relay').write_bytes(b'observer executable')
        before_identity = {'retained': 'unchanged', 'relayBinarySha256': 'old'}
        identity = {**before_identity, 'relayBinarySha256': '0x' + services['digest'](candidate / 'bin/beefy-relay')}
        (source_record.parent / 'source-before.json').write_text(json.dumps({'identity': before_identity}))
        (root / 'anchor.json').write_text('{}')
        retained = {'phase': 'ready', 'identity': identity, 'readiness': {'commonFinalized': {'height': 300, 'hash': 'root300'}}}
        retained_path = source_record.parent / 'source-after.json'
        retained_path.write_text(json.dumps(retained))
        retained_bytes = retained_path.read_bytes()
        current = {'phase': 'ready', 'identity': identity, 'readiness': {
            'commonFinalized': {'height': 301, 'hash': 'root301'},
            'authoritySets': {'current': {'length': 2, 'id': 12}, 'next': {'length': 2, 'id': 13}}}}
        def current_source(binary, directory, config, output, timeout):
            assert output != retained_path and not output.exists()
            output.write_text(json.dumps(current))
            return current
        source_state = {'sourceGate': {'name': 'source', 'status': 'RUNNING', 'deadlineAtMs': int(time.time() * 1000) + 10000},
                        'sourceBefore': {'a': {'height': 299, 'hash': 'root299'}, 'b': {'height': 299, 'hash': 'root299'}}}
        history_pins = []
        def history(config, anchor, pins, timeout):
            history_pins.append(pins)
            return {'a': {'height': 301, 'hash': 'root301'}, 'b': {'height': 301, 'hash': 'root301'}}
        with patch.dict(services['source_gate'].__globals__, native_source_state=current_source, authenticate_history=history):
            services['source_gate'](root, candidate, {'source': {'aliceRpc': 'a', 'bobRpc': 'b'}}, source_state, source_record)
        assert source_state['sourceGate']['status'] == 'PASS' and retained_path.read_bytes() == retained_bytes
        assert history_pins[-1] == {'a': {'height': 300, 'hash': 'root300'}, 'b': {'height': 300, 'hash': 'root300'}}
        original_deadline = source_state['sourceGate']['deadlineAtMs']
        source_state['sourceGate']['status'] = 'FAILED'
        rejected(lambda: services['source_gate'](root, candidate, {}, source_state, source_record))
        assert source_state['sourceGate']['status'] == 'FAILED' and source_state['sourceGate']['deadlineAtMs'] == original_deadline
        # A changed already-started actor is never implicitly bootstrapped again.
        actor_state = {'actors': {'follower': {'label': 'follower'}}, 'starts': {'follower': {'status': 'authenticated', 'pid': 7}}}
        with patch.dict(services['start_actor'].__globals__, launch_observation=lambda *args: {'pid': 8}):
            rejected(lambda: services['start_actor'](root, candidate, 'gui/0', 'follower', actor_state, recovery,
                     {'name': 'progress', 'status': 'RUNNING', 'deadlineAtMs': int(time.time() * 1000) + 10000}))
        # A durable uncertain restart intent must not issue kickstart twice.
        proof = campaign / 'warmup-supervisor-restart.json'
        proof.write_text(json.dumps({'phase': 'restart-intent-recorded'}))
        value.update(warmup={'evidence': {'restartGate': {'deadlineAtMs': 1}}})
        journal.write_text(json.dumps(value))
        with patch.object(subprocess, 'run', side_effect=AssertionError('Repeated supervisor restart')):
            observer['restart'](root, journal, proof, 'gui/0/follower', lambda *args: None, admission['require'])
        historical_campaign_sha = services['tree_digest'](campaign)
        campaign_name = 'hoodi-milestone-1-retry-1'
        campaign = root / 'campaigns' / campaign_name; campaign.mkdir(mode=0o700)
        journal = campaign / 'campaign-state.json'
        proof = campaign / 'warmup-supervisor-restart.json'
        # Immutable, compiled verify executes without ANY credential file or signer access.
        sealed = root / 'sealed'
        sealed.mkdir(mode=0o700)
        (sealed / 'ops').mkdir(mode=0o700)
        for name in ('setup-services.py', 'run-preflight.py', 'warmup-supervisor-observer.py'):
            shutil.copyfile(ops / name, sealed / 'ops' / name)
        config = {'bundle': {'path': str(sealed), 'sha256': 'selected'},
                  'source': {'aliceRpc': 'ws://127.0.0.1:1', 'bobRpc': 'ws://127.0.0.1:2'},
                  'network': {'executionWss': 'wss://invalid.example'}}
        deployment = {'anchor': {'sourceGenesis': 'genesis', 'bridgeDomain': 'domain'}}
        launch = {'identity': {'rawSpecSha256': 'raw'}}
        token_stack = {'unchanged': True}
        evm = {'roles': {'campaign': '0x01', 'follower': '0x02', 'root': '0x03'}}
        gear = {'roles': {'campaign': {'publicKey': '0x04'}, 'governance': {'publicKey': '0x05'}}}
        components = {'status': 'VERIFIED', 'bundleSha256': 'selected'}
        components_sha = hashlib.sha256(json.dumps(components).encode()).hexdigest()
        for relative, item in [('deployment.json', deployment), ('token-stack/token-stack.json', token_stack),
                               ('hoodi/addresses.json', evm), ('hoodi/gear-addresses.json', gear),
                               ('qualification/components.json', components),

                               ('bundle-transitions/hoodi-milestone-1-observer/source-after.json', launch)]:
            path = root / relative; path.parent.mkdir(mode=0o700, parents=True, exist_ok=True); path.write_text(json.dumps(item))
        native_digest = lambda item: '0x' + keccak(json.dumps(item, sort_keys=True, separators=(',', ':')).encode()).hex()
        value = {'schemaVersion': 3, 'runId': campaign_name, 'preflight': {'status': 'passed'},
                 'warmup': {'status': 'passed'}, 'status': 'warmup_passed', 'sourceGenesis': 'genesis', 'bridgeDomain': 'domain',
                 'rawSpecSha256': 'raw', 'deploymentManifestDigest': native_digest(deployment), 'tokenStackDigest': native_digest(token_stack),
                 'sourceLaunchDigest': native_digest(launch['identity']), 'accounts': {'campaignEvm': '0x01', 'followerEvm': '0x02',
                   'rootPublisherEvm': '0x03', 'campaignGear': '0x04', 'governanceGear': '0x05'}, 'endpoints': {
                     'sourceRpcDigest': '0x' + keccak(text=config['source']['aliceRpc']).hex(),
                     'witnessRpcDigest': '0x' + keccak(text=config['source']['bobRpc']).hex(),
                     'ethereumRpcDigest': '0x' + keccak(text=config['network']['executionWss']).hex(),
                     'inboundDirectory': str(root / 'inbound'), 'outboundDirectory': str(root / 'outbound/journal')}}
        journal.write_text(json.dumps(value)); journal.chmod(0o600)
        proof.write_text(json.dumps({'phase': 'native-warmup-and-real-restart-verified'}))
        artifacts = root / 'app-messages/artifacts'
        entry = artifacts / 'lib/demo/example/eth-to-vara.js'
        entry.parent.mkdir(mode=0o700, parents=True)
        entry.write_text('if(process.env.ETH_CAMPAIGN_KEY_FILE||process.env.GEAR_CAMPAIGN_SURI_FILE)process.exit(9);console.log("signer-free verified artifact")')
        entry.chmod(0o400)
        outbound_entry = entry.with_name('vara-to-eth.js')
        shutil.copyfile(entry, outbound_entry); outbound_entry.chmod(0o400)
        node = artifacts / 'bin/node'
        node.parent.mkdir(mode=0o700)
        shutil.copyfile(shutil.which('node'), node)
        node.chmod(0o500)
        artifact_manifest = artifacts / 'manifest.json'
        artifact_manifest.write_text(json.dumps({'schemaVersion': 1, 'testOnly': True, 'runId': root.name,
            'campaignName': campaign_name, 'files': {str(path.relative_to(artifacts)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in (entry, outbound_entry, node)}}))
        artifact_manifest.chmod(0o400)
        verification = sealed / 'verification.json'
        verification.write_text(json.dumps({'campaignName': campaign_name, 'applicationArtifacts': {'path': str(artifacts),
            'manifestSha256': hashlib.sha256(artifact_manifest.read_bytes()).hexdigest()}})); verification.chmod(0o400)
        selected_files = {'verification.json': hashlib.sha256(verification.read_bytes()).hexdigest(),
                          **{'ops/' + path.name: hashlib.sha256(path.read_bytes()).hexdigest() for path in (sealed / 'ops').iterdir()}}
        record = root / 'bundle-transitions/hoodi-milestone-1-observer/transition.json'
        selected_transition = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name,
            'transitionName': 'hoodi-milestone-1-observer', 'status': 'PASS', 'newBundle': config['bundle'],
            'oldBundle': {'sha256': 'historical-bundle'}, 'actors': {'follower': {'label': 'original.follower'}},
            'helperSha256': selected_files['ops/setup-services.py'],
            'sourceAfterSha256': hashlib.sha256(json.dumps(launch).encode()).hexdigest(),
            **{name: {'status': 'PASS'} for name in ('stopGate', 'sourceGate', 'progressGate')},
            'mutableFiles': {'qualification/components.json': {'new': components_sha}}}
        (root / 'supervisors').mkdir(mode=0o700)
        for mode in ('preflight', 'warmup'):
            (root / 'supervisors' / (mode + '-one-shot.plist')).write_bytes(plistlib.dumps({
                'Label': 'original.' + mode, 'ProgramArguments': [sys.executable], 'KeepAlive': False, 'RunAtLoad': True,
                'StandardOutPath': str(root / 'hoodi/old.log'), 'StandardErrorPath': str(root / 'hoodi/old.err')}))
        for relative, raw in services['one_shots'](root, sealed, selected_transition, record, campaign_name).items():
            (root / relative).write_bytes(raw)
            selected_transition['mutableFiles'][relative] = {'old': None, 'new': hashlib.sha256(raw).hexdigest()}
        record.write_text(json.dumps(selected_transition))
        sealed.chmod(0o500)
        for directory in artifacts.rglob('*'):
            if directory.is_dir():
                directory.chmod(0o500)
        artifacts.chmod(0o500)
        # A mutable PATH executable must never substitute the manifest-pinned runtime.
        hostile_path = root / 'hostile-path'
        hostile_path.mkdir(mode=0o700)
        hostile_node = hostile_path / 'node'
        hostile_node.write_text('#!/bin/sh\nexit 91\n'); hostile_node.chmod(0o700)
        context = types.SimpleNamespace(RUN=root, CONFIG=config, BUNDLE=sealed, MANIFEST={'files': selected_files},
                                       artifact=lambda *args: None, save=services['persist'], digest=services['digest'], require=admission['require'],
                                       private_text=lambda *args: (_ for _ in ()).throw(AssertionError('Verify loaded a credential')))
        flags = ['app', '--campaign-name', campaign_name, '--direction', 'eth-to-vara', '--', '--mode=verify', '--intent-id=original']
        with patch.dict(sys.modules, {'run_context': context}), patch.dict(os.environ, {'PATH': str(hostile_path)}, clear=True):
            admission['main'](flags)
            assert 'signer-free verified artifact' in (root / 'hoodi/app-eth-to-vara.log').read_text()
            assert services['tree_digest'](root / 'campaigns/hoodi-milestone-1') == historical_campaign_sha
            unqualified = list(flags); unqualified[2] = 'hoodi-milestone-1'
            rejected(lambda: admission['main'](unqualified), 'Only the qualified Hoodi campaign')
            with patch.dict(observer['main'].__globals__, supervisor=lambda *args: os.getpid()):
                rejected(lambda: observer['main'](['--campaign-name', campaign_name]), 'not a fresh attempt')
            binding_path = root / 'supervisors/hoodi-milestone-1-observer-binding.json'
            bound_bytes = binding_path.read_bytes()
            binding = json.loads(bound_bytes)
            binding['transition'] = str(root / 'unbound-PASS.json')
            binding_path.write_text(json.dumps(binding))
            rejected(lambda: admission['main'](flags), 'transition/bundle binding changed')
            binding_path.write_bytes(bound_bytes)
            held = {**selected_transition, 'status': 'HOLD'}
            record.write_text(json.dumps(held))
            rejected(lambda: admission['main'](flags), 'Fenced bound transition has not passed')
            record.write_text(json.dumps(selected_transition))
            # A bound but failed recovery cannot fall back to the predecessor's otherwise passing fixture.
            recovery_record = root / 'bundle-transitions/hoodi-milestone-1-recovery-1/transition.json'
            recovery_record.parent.mkdir(mode=0o700)
            recovery_record.write_text(json.dumps({**held, 'transitionName': 'hoodi-milestone-1-recovery-1'}))
            binding.update(transition=str(recovery_record), transitionName='hoodi-milestone-1-recovery-1')
            binding_path.write_text(json.dumps(binding))
            rejected(lambda: admission['main'](flags), 'Fenced bound transition has not passed')
            binding_path.write_bytes(bound_bytes)
            raw_manifest = artifact_manifest.read_bytes()
            artifact_manifest.chmod(0o600); artifact_manifest.write_bytes(raw_manifest + b'\n'); artifact_manifest.chmod(0o400)
            rejected(lambda: admission['main'](flags), 'App artifact manifest differs from selected qualification')
            artifact_manifest.chmod(0o600); artifact_manifest.write_bytes(raw_manifest); artifact_manifest.chmod(0o400)
            component_path = root / 'qualification/components.json'
            component_path.write_text(json.dumps({**components, 'unqualified': True}))
            rejected(lambda: admission['main'](flags), 'Selected component qualification changed')
            component_path.write_text(json.dumps(components))
            value['sourceGenesis'] = 'substituted'
            journal.write_text(json.dumps(value))
            rejected(lambda: admission['main'](flags))
            value['sourceGenesis'] = 'genesis'; journal.write_text(json.dumps(value))
            original_entry = entry.read_bytes()
            entry.chmod(0o600); entry.write_text('throw Error("changed executable")'); entry.chmod(0o400)
            rejected(lambda: admission['main'](flags))
            entry.chmod(0o600); entry.write_bytes(original_entry); entry.chmod(0o400)
        # A newly qualified immutable directory is selected without renaming or touching the old artifacts.
        old_tree = services['tree_digest'](artifacts)
        retry_artifacts = artifacts.with_name('artifacts-retry-1')
        shutil.copytree(artifacts, retry_artifacts)
        retry_app_manifest = retry_artifacts / 'manifest.json'
        app_identity = json.loads(raw_manifest)
        app_identity['campaignName'] = 'hoodi-milestone-1-retry-2'
        def store_retry_manifest(value):
            retry_app_manifest.chmod(0o600)
            retry_app_manifest.write_text(json.dumps(value)); retry_app_manifest.chmod(0o400)
        store_retry_manifest(app_identity)
        retry_sealed = root / 'sealed-retry'; retry_sealed.mkdir(mode=0o700)
        retry_verification = retry_sealed / 'verification.json'
        def qualify_retry(value=app_identity, name='hoodi-milestone-1-retry-2'):
            store_retry_manifest(value)
            retry_verification.write_text(json.dumps({'campaignName': name, 'applicationArtifacts': {
                'path': str(retry_artifacts), 'manifestSha256': services['digest'](retry_app_manifest)}}))
            return {'files': {'verification.json': services['digest'](retry_verification)}}
        retry_manifest = qualify_retry()
        services['qualify_application_rebind'](root, sealed, retry_sealed, {'files': selected_files}, retry_manifest, admission)
        assert admission['qualified_campaign'](retry_sealed, retry_manifest['files']['verification.json']) == 'hoodi-milestone-1-retry-2'
        for name in ('hoodi-milestone-1-retry-0', 'hoodi-milestone-1-retry-01', 'hoodi-milestone-1-recovery-1', 'other', None,
                     'hoodi-milestone-1-retry-' + '1' * 64):
            invalid = qualify_retry(name=name)
            rejected(lambda: admission['qualified_campaign'](retry_sealed, invalid['files']['verification.json']), 'Invalid qualified')
        retry_manifest = qualify_retry()
        rejected(lambda: admission['qualified_campaign'](retry_sealed, '00' * 32), 'qualification changed')
        for field, value in (('runId', 'another-run'), ('testOnly', False), ('unqualifiedField', True)):
            altered = {**app_identity, field: value}
            altered_manifest = qualify_retry(altered)
            rejected(lambda: services['qualify_application_rebind'](root, sealed, retry_sealed,
                {'files': selected_files}, altered_manifest, admission))
        retry_entry = retry_artifacts / 'lib/demo/example/eth-to-vara.js'
        retry_entry.chmod(0o600); retry_entry.write_bytes(original_entry + b'\n// different code'); retry_entry.chmod(0o400)
        altered = {**app_identity, 'files': {**app_identity['files'],
            'lib/demo/example/eth-to-vara.js': services['digest'](retry_entry)}}
        altered_manifest = qualify_retry(altered)
        rejected(lambda: services['qualify_application_rebind'](root, sealed, retry_sealed,
            {'files': selected_files}, altered_manifest, admission), 'beyond campaignName')
        retry_entry.chmod(0o600); retry_entry.write_bytes(original_entry); retry_entry.chmod(0o400)
        retry_manifest = qualify_retry()
        services['qualify_application_rebind'](root, sealed, retry_sealed, {'files': selected_files}, retry_manifest, admission)
        assert services['tree_digest'](artifacts) == old_tree

        new_artifacts = artifacts.with_name('artifacts-recovery-1')
        shutil.copytree(artifacts, new_artifacts)
        verified = json.loads(verification.read_bytes())
        for target in (str(new_artifacts), str(root / 'outside-app'), str(new_artifacts / '..' / new_artifacts.name)):
            verified['applicationArtifacts']['path'] = target
            verification.chmod(0o600); verification.write_text(json.dumps(verified)); verification.chmod(0o400)
            qualified_sha = hashlib.sha256(verification.read_bytes()).hexdigest()
            if target == str(new_artifacts):
                assert admission['app_artifact'](root, campaign_name, 'eth-to-vara', sealed, qualified_sha) == (
                    new_artifacts / 'bin/node', new_artifacts / 'lib/demo/example/eth-to-vara.js')
            else:
                rejected(lambda: admission['app_artifact'](root, campaign_name, 'eth-to-vara', sealed, qualified_sha))
        alias = artifacts.with_name('artifact-alias'); alias.symlink_to(new_artifacts)
        verified['applicationArtifacts']['path'] = str(alias)
        verification.chmod(0o600); verification.write_text(json.dumps(verified)); verification.chmod(0o400)
        rejected(lambda: admission['app_artifact'](root, campaign_name, 'eth-to-vara', sealed,
                 hashlib.sha256(verification.read_bytes()).hexdigest()), 'Symlink in admission path')
        assert services['tree_digest'](artifacts) == old_tree
        for flags in (['--mode=verify', '--mode=send', '--intent-id=x'], ['--mode=send', '--intent-id=x', '--private-key=secret'],
                      ['--mode=send', '--intent-id=x', '--resume=true']):
            rejected(lambda flags=flags: admission['app_arguments'](flags))
        # Exercise the actual nested runner -> pinned Node FD lifetime, including abrupt runner death.
        import signal
        lifetime_log = root / 'native-child-lifetime.log'
        finish = root / 'finish-native-child'
        javascript = "const fs=require('fs');const fd=Number(process.env.BEEFY_CAMPAIGN_LOCK_FD);console.log('native-held',process.pid,fs.fstatSync(fd).ino);const timer=setInterval(()=>{if(fs.existsSync(process.argv[1])){clearInterval(timer);process.exit(0)}},20)"
        python_child = "import os,runpy,sys;from pathlib import Path;ops=runpy.run_path(sys.argv[1]);root=Path(sys.argv[2]);lock=ops['campaign_lock'](root);ops['run_child']([sys.argv[3],'-e',sys.argv[4],sys.argv[5]],root,{'BEEFY_CAMPAIGN_LOCK_FD':str(lock.fileno())},lock,Path(sys.argv[6]))"
        wrapper = subprocess.Popen([sys.executable, '-c', python_child, str((ops / 'run-preflight.py').resolve()),
            str(root), str(node), javascript, str(finish), str(lifetime_log)], start_new_session=True,
            env={key: os.environ[key] for key in ('PATH', 'HOME') if key in os.environ},
            stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            deadline = time.monotonic() + 10
            while not lifetime_log.exists() or 'native-held' not in lifetime_log.read_text():
                assert wrapper.poll() is None and time.monotonic() < deadline, 'Native child did not inherit the lock'
                time.sleep(0.02)
            row = lifetime_log.read_text().split()
            assert int(row[2]) == (root / 'bounded-campaign.lock').stat().st_ino
            wrapper.kill(); assert wrapper.wait(timeout=5) == -signal.SIGKILL
            try:
                admission['campaign_lock'](root)
            except BlockingIOError:
                pass
            else:
                raise AssertionError('Runner death released the still-live native Node campaign lock')
            finish.touch()
            while True:
                try:
                    released = admission['campaign_lock'](root)
                except BlockingIOError:
                    assert time.monotonic() < deadline, 'Exited native child retained ownership'
                    time.sleep(0.02)
                else:
                    released.close()
                    break
        finally:
            try:
                os.killpg(wrapper.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            wrapper.wait(timeout=5)
            wrapper.stderr.close()

    print('Milestone admission passed: named/private identity, inherited native lock, CAS, fencing/deadlines, immutable signer-free app verify')


def check_retry_campaign_admission():
    import copy
    import plistlib
    services = runpy.run_path(str(Path(__file__).with_name('setup-services.py')))
    admission = runpy.run_path(str(Path(__file__).with_name('run-preflight.py')))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary).resolve() / services['RETAINED_RUN']; root.mkdir(mode=0o700)
        old_name, fresh_name = 'hoodi-milestone-1', 'hoodi-milestone-1-retry-1'
        campaign = root / 'campaigns' / old_name; campaign.mkdir(mode=0o700, parents=True)
        journal = campaign / 'campaign-state.json'
        original = services['json_bytes']({'schemaVersion': 3, 'runId': old_name, 'status': 'failed_before_t0',
            'preflight': {'status': 'failed', 'evidence': {'startedAtMs': 1000, 'deadlineAtMs': 2641000}},
            'warmup': {'status': 'pending'}, 'originalNonce': '0', 'originalTransactionHash': 'retained-original'})
        journal.write_bytes(original); journal.chmod(0o600)
        old_tree = services['tree_digest'](campaign)
        supervisors = root / 'supervisors'; supervisors.mkdir(mode=0o700)
        for mode in ('preflight', 'warmup'):
            for name in (mode + '-one-shot', old_name + '-' + mode):
                (supervisors / (name + '.plist')).write_bytes(plistlib.dumps({'Label': 'original.' + name}))
        evidence_path = root / 'original-intent-reconciliation.json'
        evidence = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'status': 'VERIFIED',
            'unresolvedOriginals': [], 'namedCampaigns': {old_name: {'status': 'VERIFIED',
                'campaignTreeSha256': old_tree, 'unresolvedOriginals': [], 'finalizedOriginal': 'retained-original'}}}
        def pin(value):
            evidence_path.write_bytes(services['json_bytes'](value))
            return {'path': str(evidence_path), 'sha256': services['digest'](evidence_path)}
        binding = pin(evidence)
        def admit(binding=binding):
            return services['recovery_unstarted'](root, 'gui/0', fresh_name, binding)
        with patch.dict(services['recovery_unstarted'].__globals__, launch_observation=lambda *args: None), \
             patch.object(subprocess, 'run', side_effect=AssertionError('Admission must never fence or launch jobs')):
            assert admit() == {'campaigns/' + old_name: old_tree}
            assert admission['campaign_directory'](root, fresh_name, 'preflight') == root / 'campaigns' / fresh_name
            rejected(lambda: admission['campaign_directory'](root, old_name, 'preflight'), 'terminal')
            rejected(lambda: admission['campaign_directory'](root, old_name, 'warmup'), 'not passed')
            for change in ('unrecorded', 'extra-record', 'unresolved', 'unverified', 'wrong-hash'):
                altered = copy.deepcopy(evidence)
                if change == 'unrecorded': altered['namedCampaigns'] = {}
                elif change == 'extra-record': altered['namedCampaigns'][fresh_name] = copy.deepcopy(evidence['namedCampaigns'][old_name])
                elif change == 'unresolved': altered['namedCampaigns'][old_name]['unresolvedOriginals'] = ['retained-original']
                elif change == 'unverified': altered['namedCampaigns'][old_name]['status'] = 'HOLD'
                else: altered['namedCampaigns'][old_name]['campaignTreeSha256'] = '00' * 32
                rejected(lambda: admit(pin(altered)))
            binding = pin(evidence)
            stale = {**binding, 'sha256': '00' * 32}
            rejected(lambda: admit(stale), 'Pinned continuation evidence changed')
            journal.write_bytes(original + b'\n'); rejected(lambda: admit(binding), 'changed'); journal.write_bytes(original)
            for status in ('running', 'warmup_passed'):
                altered_journal = json.loads(original); altered_journal['status'] = status
                journal.write_bytes(services['json_bytes'](altered_journal))
                rejected(lambda: admit(binding), 'not terminal'); journal.write_bytes(original)
            fresh = root / 'campaigns' / fresh_name; fresh.mkdir(mode=0o700)
            rejected(lambda: admit(binding), 'already'); fresh.rmdir()
            extra = root / 'campaigns/hoodi-milestone-1-retry-2'; extra.mkdir(mode=0o700)
            rejected(lambda: admit(binding), 'journal is missing'); extra.rmdir()
            alias = root / 'campaigns/hoodi-milestone-1-retry-3'; alias.symlink_to(campaign)
            rejected(lambda: admit(binding), 'Symlink'); alias.unlink()
            nested = campaign / 'alias'; nested.symlink_to(journal)
            rejected(lambda: admit(binding), 'Unexpected retained tree entry'); nested.unlink()
            for relative in ('app-messages/intents', 'app-messages/deployment.json'):
                path = root / relative; path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                if path.suffix: path.write_text('{}')
                else: path.mkdir(mode=0o700)
                rejected(lambda: admit(binding), 'Application preparation/intent')
                if path.suffix: path.unlink()
                else: path.rmdir()
            with patch.dict(services['recovery_unstarted'].__globals__, launch_observation=lambda *args: {'pid': 7}):
                rejected(lambda: admit(binding), 'one-shot is loaded')
            assert admit(binding) == {'campaigns/' + old_name: old_tree}
            assert journal.read_bytes() == original and services['tree_digest'](campaign) == old_tree
    print('Distinct retry admission passed: reconciled terminal history only, exact inventory, private trees, original deadlines, no rerun or live mutation')



def check_milestone_qualification():
    services = runpy.run_path(str(Path(__file__).with_name('setup-services.py')))
    commands = [
        'python3 tools/beefy-relay/ops/test_ops.py',
        'cargo nextest run -p beefy-relay',
        'cargo build --locked -p ping --release',
        'forge build --root js/bridge-js/js-test/contracts --force --no-cache',
        'forge test --root js/bridge-js/js-test/contracts --match-contract MessageHandlerTest -vvv',
        'yarn workspace @gear-js/bridge typecheck',
        'yarn workspace @gear-js/bridge test test/vara-to-eth.test.ts',
        'yarn workspace @gear-js/bridge test test/eth-to-vara.test.ts',
        'yarn workspace @gear-js/bridge build:examples',
    ]
    with tempfile.TemporaryDirectory() as temporary:
        bundle = Path(temporary)
        log = bundle / 'passed.log'; log.write_text('owned offline check fixture\n')
        log_sha = hashlib.sha256(log.read_bytes()).hexdigest()
        evidence = {'schemaVersion': 1, 'status': 'VERIFIED', 'checks': [
            {'name': name, 'command': name, 'exitCode': 0, 'logPath': str(log), 'logSha256': log_sha}
            for name in ('cargo-tests', 'forge-tests', 'full-release-build', 'historical-recovery')],
            'sourceFiles': {str(log): log_sha}, 'binaries': {}}
        manifest = {'files': {}, 'binaries': {}}
        (bundle / 'verification.json').write_text(json.dumps(evidence))
        rejected(lambda: services['qualify_candidate'](bundle, manifest), 'Milestone-1 offline checks are missing')
        evidence['checks'] += [dict(evidence['checks'][0], name='milestone-' + str(index), command=command)
                               for index, command in enumerate(commands)]
        (bundle / 'verification.json').write_text(json.dumps(evidence))
        assert services['qualify_candidate'](bundle, manifest)['status'] == 'VERIFIED'
        # A build without the locked dependency contract is not the approved check.
        evidence['checks'][6]['command'] = 'cargo build -p ping --release'
        (bundle / 'verification.json').write_text(json.dumps(evidence))
        rejected(lambda: services['qualify_candidate'](bundle, manifest), 'Milestone-1 offline checks are missing')
    print('Milestone qualification passed: all nine exact offline commands required before cutover')



def check_transition_review():
    import time
    ops = Path(__file__).parent
    services = runpy.run_path(str(ops / 'setup-services.py'))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary).resolve()
        bundle = root / 'bundle'
        (bundle / 'bin').mkdir(parents=True)
        files = {}
        for name in ('gear', 'beefy-relay', 'relayer', 'checkpoints-tool'):
            path = bundle / 'bin' / name
            path.write_bytes(name.encode()); path.chmod(0o500)
            files['bin/' + name] = hashlib.sha256(path.read_bytes()).hexdigest()
        manifest = {'schemaVersion': 1, 'testOnly': True, 'files': files,
                    'binaries': {name: 'bin/' + name for name in ('gear', 'beefy-relay', 'relayer', 'checkpoints-tool')}}
        descriptor = bundle / 'bundle.json'
        descriptor.write_text(json.dumps(manifest)); descriptor.chmod(0o400)
        (bundle / 'bin').chmod(0o500)
        expected = hashlib.sha256(descriptor.read_bytes()).hexdigest()
        rejected(lambda: services['authenticate_bundle'](bundle, expected), 'Bundle is not immutable')
        bundle.chmod(0o500)
        assert services['authenticate_bundle'](bundle, expected)['files'] == files

        # The last fencing observation can consume the last millisecond of the ORIGINAL stop gate.
        clock = [0]
        state = {'actors': {name: {'label': name, 'pid': 7} for name in services['ACTORS']}, 'stopIntents': {},
                 'checkpoint': 'checkpoint', 'originalProcesses': [],
                 'stopGate': {'name': 'stop', 'status': 'RUNNING', 'deadlineAtMs': 100}}
        def late_process_table(timeout):
            clock[0] = 101
            return {}
        with patch.dict(services['stop_barrier'].__globals__, clock_ms=lambda: clock[0],
                        launch_observation=lambda *args: None, process_table=late_process_table):
            rejected(lambda: services['stop_barrier'](root, 'gui/0', state, root / 'late-stop.json', state['stopGate']),
                     'deadline expired')
        assert json.loads((root / 'late-stop.json').read_text())['stopGate']['status'] == 'FAILED'

        # A loaded but unrecorded job is not this transition's original dispatched actor.
        gate = {'name': 'source', 'status': 'RUNNING', 'deadlineAtMs': 10000}
        process = {'pid': 7, 'ppid': 1, 'pgid': 7, 'started': 'original start',
                   'command': str(bundle / 'bin/beefy-relay') + ' tokens-follow'}
        state = {'actors': {'follower': {'label': 'follower'}}, 'starts': {}}
        with patch.dict(services['start_actor'].__globals__, clock_ms=lambda: 1,
                        launch_observation=lambda *args: {'pid': 7}, process_table=lambda *args: {7: process}):
            rejected(lambda: services['start_actor'](root, bundle, 'gui/0', 'follower', state, root / 'actor.json', gate),
                     'Unrecorded actor')
        assert not state['starts']
        state['starts']['follower'] = {'status': 'authenticated', 'pid': 7, 'process': process}
        changed = {**process, 'started': 'reused PID start'}
        with patch.dict(services['start_actor'].__globals__, clock_ms=lambda: 1,
                        launch_observation=lambda *args: {'pid': 7}, process_table=lambda *args: {7: changed}):
            rejected(lambda: services['start_actor'](root, bundle, 'gui/0', 'follower', state, root / 'actor.json', gate),
                     'process identity changed')
        assert state['starts']['follower']['process']['started'] == 'original start'
        # A real recorded identity reconciles in place; bootstrap and original dispatch time do not reset.
        import plistlib
        (root / 'supervisors').mkdir(mode=0o700)
        (root / 'supervisors/follower.plist').write_bytes(plistlib.dumps({'ProgramArguments': [
            '/python', str(bundle / 'ops/setup-services.py'), 'run', 'follower']}))
        state['starts']['follower'].update(atMs=0, authenticatedAtMs=0)
        with patch.dict(services['start_actor'].__globals__, clock_ms=lambda: 1,
                        launch_observation=lambda *args: {'pid': 7}, process_table=lambda *args: {7: process}), \
             patch.object(subprocess, 'run', side_effect=AssertionError('Authenticated actor was bootstrapped again')):
            services['start_actor'](root, bundle, 'gui/0', 'follower', state, root / 'actor.json', gate)
        assert state['starts']['follower']['atMs'] == state['starts']['follower']['authenticatedAtMs'] == 0
        wrong_command = {**process, 'command': str(bundle / 'ops/run-preflight.py') + ' preflight'}
        with patch.dict(services['start_actor'].__globals__, clock_ms=lambda: 1,
                        launch_observation=lambda *args: {'pid': 7}, process_table=lambda *args: {7: wrong_command}):
            rejected(lambda: services['start_actor'](root, bundle, 'gui/0', 'follower', state, root / 'actor.json', gate),
                     'exact candidate sealed wrapper/executable')

        # A dispatched wrapper is not readiness: wait for its SAME process to exec the native actor.
        for scenario in ('exec', 'process-group', 'timeout', 'pid-change', 'bad-group', 'late-observation'):
            clock = [0]
            startup_gate = {'name': 'source', 'status': 'RUNNING', 'deadlineAtMs': 500}
            startup = {'actors': {'follower': {'label': 'follower'}},
                       'starts': {'follower': {'status': 'intent', 'atMs': -1}}}
            startup_record = root / (scenario + '.json')
            def startup_observation(*args):
                return {'pid': 8 if scenario == 'pid-change' and clock[0] >= 100 else 7}
            def startup_processes(*args):
                current = dict(process)
                if clock[0] < 200 or scenario == 'timeout':
                    current['command'] = '/platform/python-real /sealed/wrapper.py'
                if scenario == 'process-group' and clock[0] < 100:
                    current['pgid'] = 1
                if scenario == 'bad-group' and clock[0] >= 100:
                    current['pgid'] = 9
                if scenario == 'late-observation' and clock[0] >= 200:
                    clock[0] = startup_gate['deadlineAtMs']
                return {7: current}
            def startup_wait(seconds):
                clock[0] += round(seconds * 1000)
            with patch.dict(services['start_actor'].__globals__, clock_ms=lambda: clock[0],
                            launch_observation=startup_observation, process_table=startup_processes), \
                 patch.object(time, 'sleep', startup_wait), \
                 patch.object(subprocess, 'run', side_effect=AssertionError('Dispatched actor was bootstrapped again')):
                if scenario in ('exec', 'process-group'):
                    services['start_actor'](root, bundle, 'gui/0', 'follower', startup, startup_record, startup_gate)
                    assert startup['starts']['follower']['status'] == 'authenticated'
                    assert startup['starts']['follower']['process']['command'] == process['command']
                    assert startup['starts']['follower']['authenticatedAtMs'] == 200
                else:
                    rejected(lambda: services['start_actor'](root, bundle, 'gui/0', 'follower', startup, startup_record, startup_gate),
                             'identity changed' if scenario in ('pid-change', 'bad-group') else 'deadline expired')
                    assert startup['starts']['follower']['status'] == 'intent'
            assert startup_gate['deadlineAtMs'] == 500 and startup['starts']['follower']['atMs'] == -1
            assert json.loads(startup_record.read_text())['starts']['follower']['pid'] == 7
            assert startup['starts']['follower']['process']['started'] == 'original start'
            if scenario == 'timeout':
                # Resume cannot replace a PID first pinned while its wrapper was still running.
                with patch.dict(services['start_actor'].__globals__, clock_ms=lambda: 1,
                                launch_observation=lambda *args: {'pid': 8}):
                    rejected(lambda: services['start_actor'](root, bundle, 'gui/0', 'follower', startup, startup_record,
                             {'name': 'source', 'status': 'RUNNING', 'deadlineAtMs': 500}), 'identity changed')


        # Incomplete native saves remain preserved in a frozen private snapshot, not repaired by setup.
        for relative in ('follower', 'inbound', 'outbound/journal', 'checkpoint'):
            (root / relative).mkdir(mode=0o700, parents=True, exist_ok=True)
        native_save = root / 'follower/state.json.tmp'
        native_save.write_bytes(b'incomplete native save'); native_save.chmod(0o600)
        snapshot = root / 'before-runtime'
        state = {}
        services['private_snapshot'](root, snapshot, state, root / 'snapshot.json')
        assert state['unfinishedNativeSaves'] == ['follower/state.json.tmp']
        assert native_save.read_bytes() == b'incomplete native save' and native_save.stat().st_mode & 0o777 == 0o600
        frozen = snapshot / 'follower/state.json.tmp'
        assert frozen.read_bytes() == b'incomplete native save' and frozen.stat().st_mode & 0o777 == 0o400
        services['private_snapshot'](root, snapshot, state, root / 'snapshot.json')
    print('Transition review passed: immutable bundle root, deadline expiry, recorded process identity, retained native saves')


def check_repeatable_continuation():
    import copy
    import plistlib
    services = runpy.run_path(str(Path(__file__).with_name('setup-services.py')))
    admission = runpy.run_path(str(Path(__file__).with_name('run-preflight.py')))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary).resolve() / services['RETAINED_RUN']; root.mkdir(mode=0o700)
        def store(path, value):
            path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
            raw = value if isinstance(value, bytes) else services['json_bytes'](value)
            path.write_bytes(raw); path.chmod(0o600)
            return services['digest'](path)
        bundle = root / 'selected-bundle'
        for name in ('gear', 'beefy-relay', 'relayer', 'checkpoints-tool'):
            store(bundle / 'bin' / name, ('sealed fixture ' + name).encode())
        for name in ('setup-services.py', 'run-preflight.py', 'warmup-supervisor-observer.py'):
            raw = Path(__file__).with_name(name).read_bytes()
            if name == 'setup-services.py':
                raw = raw.replace(("GEAR_SHA = '" + services['GEAR_SHA'] + "'").encode(),
                    ("GEAR_SHA = '" + services['digest'](bundle / 'bin/gear') + "'").encode())
            store(bundle / 'ops' / name, raw)
        store(bundle / 'verification.json', {'campaignName': 'hoodi-milestone-1', 'applicationArtifacts': {}})
        manifest = {'schemaVersion': 1, 'testOnly': True, 'solidity': {},
            'binaries': {name: 'bin/' + name for name in ('gear', 'beefy-relay', 'relayer', 'checkpoints-tool')},
            'files': {str(path.relative_to(bundle)): services['digest'](path) for path in bundle.rglob('*') if path.is_file()}}
        selected = {'path': str(bundle), 'sha256': store(bundle / 'bundle.json', manifest)}
        for path in bundle.rglob('*'):
            path.chmod(0o400 if path.is_file() else 0o500)
        bundle.chmod(0o500)
        assert services['authenticate_bundle'](bundle, selected['sha256']) == manifest
        retry_bundle = root / 'retry-bundle'
        shutil.copytree(bundle, retry_bundle)
        retry_verification = retry_bundle / 'verification.json'
        retry_verification.chmod(0o600)
        store(retry_verification, {'campaignName': 'hoodi-milestone-1-retry-1', 'applicationArtifacts': {}})
        retry_manifest = {**manifest, 'files': {**manifest['files'], 'verification.json': services['digest'](retry_verification)}}
        retry_descriptor = retry_bundle / 'bundle.json'; retry_descriptor.chmod(0o600)
        retry_selected = {'path': str(retry_bundle), 'sha256': store(retry_descriptor, retry_manifest)}
        retry_verification.chmod(0o400); retry_descriptor.chmod(0o400)
        config = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'network': {'chainId': 560048},
                  'bundle': selected, 'funding': {'address': '0x' + 'ab' * 20}}
        store(root / 'run.json', config)
        store(root / 'source-chain/supervisor-plan.json', {'bundleSha256': selected['sha256']})
        store(root / 'qualification/components.json', {'status': 'VERIFIED', 'bundleSha256': selected['sha256']})
        launch_sha = store(root / 'source-chain/launch-state.json', {'identity': {'runtime': 'unchanged'}})
        store(root / 'campaign/campaign-state.json', {'status': 'failed_before_t0'})
        legacy = root / 'bundle-transitions/checkpoint-replay-1/transition.json'
        store(legacy, {'status': 'PASS', 'originalHistory': True})
        authorization_path = root / 'continuation-authorization.json'
        authorization = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name,
            'routineHoodiFixesRestartsDeploymentsAndSeparateTestAttemptsAuthorized': True,
            'refillAndDistributionAuthorized': True, 'failedHistoryPreserved': True,
            'distributionAddress': config['funding']['address'],
            **{field: False for field in ('mainnetSigningAuthorized', 'mainnetActivationAuthorized',
                                         'resetFailedDeadlines', 'replaceUnresolvedSignedIntents')}}
        authorization_pin = {'path': str(authorization_path), 'sha256': store(authorization_path, authorization)}
        actors = {name: {'label': 'original.' + name, 'pid': index + 10} for index, name in enumerate(services['ACTORS'])}
        for name, actor in actors.items():
            native = 'gear' if name in ('alice', 'bob') else 'beefy-relay' if name == 'follower' else 'relayer'
            actor['process'] = {'pid': actor['pid'], 'ppid': 1, 'pgid': actor['pid'], 'started': 'historical',
                'command': str(bundle / 'bin' / native) + ' --owner ' + str(root / name)}
            store(root / 'supervisors' / (name + '.plist'), plistlib.dumps({'Label': actor['label']}))
        for mode in ('preflight', 'warmup'):
            store(root / 'supervisors' / (mode + '-one-shot.plist'), plistlib.dumps({'Label': 'original.' + mode,
                'ProgramArguments': [sys.executable], 'KeepAlive': False, 'RunAtLoad': True,
                'StandardOutPath': str(root / 'hoodi/old.log'), 'StandardErrorPath': str(root / 'hoodi/old.err')}))
        for relative in ('follower', 'inbound', 'outbound/journal', 'checkpoint'):
            store(root / relative / 'state.json', {'roots': {}, 'original-intent': relative})
        for relative in ('app-messages/artifacts', 'app-messages/artifacts-recovery-1'):
            store(root / relative / 'retained', b'immutable app bytes')
        store(root / 'app-messages/public-recovery-1-result.json',
              {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'status': 'HOODI_MILESTONE_1_HOLD'})
        def bind_attempt(record, state, failed=False):
            for relative, pins in state['mutableFiles'].items():
                services['cas_replace'](root / relative, pins['old'], pins['new'],
                    (record.parent / 'after' / relative).read_bytes())
            services['private_snapshot'](root, record.parent / 'before-runtime', state, root / 'snapshot-check.json')
            state['sourceAfterSha256'] = store(record.parent / 'source-after.json', {'phase': 'ready', 'identity': {'runtime': 'unchanged'}})
            now = state['preparedAtMs']
            state['sourceGate'].update(status='PASS', startedAtMs=now + 1000, deadlineAtMs=now + 121000)
            state['stopGate'].update(status='PASS', jobsAbsent=True, wrapperAndNativeExited=True)
            state['status'], state['phase'] = 'HOLD', 'descriptor-and-supervisors-bound'
            state['progressGate']['status'] = 'FAILED' if failed else 'PENDING'
            store(record, state)
        def attempt(name, predecessor=None, failed=False, progress_cap=services['CONTINUATION_PROGRESS_MS'], unapplied=False, candidate_pin=None):
            record = root / 'bundle-transitions' / name / 'transition.json'
            now = services['clock_ms']() - (31000 if unapplied and failed else 0)
            new_bundle = candidate_pin or selected
            candidate = Path(new_bundle['path'])
            candidate_manifest = services['authenticate_bundle'](candidate, new_bundle['sha256'])
            with patch.dict(services['bundle_diff'].__globals__, GEAR_SHA=manifest['files']['bin/gear']):
                diff = services['bundle_diff'](manifest, candidate_manifest, predecessor is not None)
            state = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'transitionName': name,
                'status': 'RUNNING', 'phase': 'prepared', 'oldBundle': copy.deepcopy(selected), 'newBundle': copy.deepcopy(new_bundle),
                'helperSha256': candidate_manifest['files']['ops/setup-services.py'], 'actors': copy.deepcopy(actors),
                'originalProcesses': [actor['process'] for actor in actors.values()], 'starts': {}, 'stopIntents': {},
                'preparedAtMs': now, 'lastWallMs': now, 'allowlistedArtifactDiff': diff,
                'preservedFiles': {'source-chain/launch-state.json': launch_sha}, 'mutableFiles': {},
                'originalCampaignTreeSha256': services['tree_digest'](root / 'campaign'),
                'sourceBefore': {}, 'sourceBeforeSha256': store(record.parent / 'source-before.json', {'identity': {'runtime': 'unchanged'}}),
                'checkpoint': 'checkpoint', 'embeddedPrograms': {}, 'originalIntentReconciliation': {},
                'executionBefore': {'beefyBlock': 7, 'queueBlock': 5}, 'checkpointBefore': {'slot': 10},
                'campaignsLaunched': False, 'nativeJournalManuallyEdited': False,
                'fundingPerformedByTransition': False, 'identitiesReset': False,
                'stopGate': {'name': 'stop', 'status': 'RUNNING', 'startedAtMs': now, 'deadlineAtMs': now + 30000},
                'sourceGate': {'name': 'source', 'status': 'PENDING', 'absoluteCapAtMs': now + 150000},
                'progressGate': {'name': 'applied-progress', 'status': 'PENDING', 'deadlineAtMs': now + progress_cap}}
            if predecessor:
                state.update(predecessor=predecessor, continuationAuthorization=authorization_pin, fundingBefore={'accounts': {}},
                    rootScanTarget={'height': 45, 'hash': '0x' + '45' * 32},
                    preservedArtifactTrees={str(path.relative_to(root)): services['tree_digest'](path)
                        for path in (root / 'app-messages').iterdir() if path.is_dir()},
                    historyTrees={str(path.relative_to(root)): services['tree_digest'](path)
                        for path in (root / 'bundle-transitions').iterdir() if path != record.parent})
            replacements = {relative: (root / relative).read_bytes() for relative in
                services['load'](Path(predecessor['path']))['mutableFiles']} if predecessor else {}
            replacements.update(services['one_shots'](root, candidate, state, record,
                admission['qualified_campaign'](candidate, candidate_manifest['files']['verification.json'])))
            for name in services['ACTORS']:
                replacements['supervisors/' + name + '.plist'] = (root / 'supervisors' / (name + '.plist')).read_bytes()
            for relative in ('run.json', 'source-chain/supervisor-plan.json', 'qualification/components.json'):
                replacements[relative] = (root / relative).read_bytes()
            if candidate_pin:
                replacements['run.json'] = services['json_bytes']({**services['load'](root / 'run.json'), 'bundle': new_bundle})
            for relative, raw in replacements.items():
                path = root / relative
                state['mutableFiles'][relative] = {'old': services['digest'](path) if path.exists() else None,
                    'new': hashlib.sha256(raw).hexdigest()}
                if path.exists():
                    before = record.parent / 'before' / relative
                    store(before, path.read_bytes()); before.chmod(0o400)
                after = record.parent / 'after' / relative
                store(after, raw); after.chmod(0o400)
            if predecessor:
                path = record.parent / 'recovery-authorization.json'
                state['recoveryAuthorizationSha256'] = store(path, services['recovery_identity'](state)); path.chmod(0o400)
            if unapplied:
                if failed:
                    state['status'] = 'HOLD'
                    state['stopGate'].update(status='FAILED', reason='Original stop deadline expired before first command')
                    state['stopIntents']['follower'] = {'atMs': now, 'original': copy.deepcopy(actors['follower'])}
                store(record, state)
            else:
                bind_attempt(record, state, failed)
            return record, state
        original, original_state = attempt(services['TRANSITION'], failed=True, progress_cap=44 * 60000)
        original_state['sourceGate']['status'], original_state['progressGate']['status'] = 'FAILED', 'PENDING'
        store(original, original_state)
        _, original_pin = services['recovery_predecessor'](root, selected, {'path': str(original), 'sha256': services['digest'](original)})
        previous_record, previous = attempt('hoodi-milestone-1-recovery-1', original_pin, failed=True, progress_cap=44 * 60000)
        previous['progressGate'].update(status='HOLD', reason='Native history audit exited nonzero')
        store(previous_record, previous)
        for change in ('pending', 'running', 'empty-reason', 'unready-source', 'running-owner'):
            altered = copy.deepcopy(previous)
            if change in ('pending', 'running'): altered['progressGate']['status'] = change.upper()
            elif change == 'empty-reason': altered['progressGate']['reason'] = ''
            elif change == 'unready-source': altered['sourceGate']['status'] = 'HOLD'
            else: altered['status'] = 'RUNNING'
            store(previous_record, altered)
            rejected(lambda: services['recovery_predecessor'](root, selected,
                {'path': str(previous_record), 'sha256': services['digest'](previous_record)}), 'resume its existing owner')
        store(previous_record, previous)
        failure_hashes = {str(path): services['digest'](path) for path in (original, previous_record)}
        _, previous_pin = services['recovery_predecessor'](root, selected,
            {'path': str(previous_record), 'sha256': services['digest'](previous_record)})
        # No stale-baseline PID or frozen worker-file assumption: running actors can restart and state can advance.
        table = {}
        for name, actor in actors.items():
            process = {**actor['process'], 'pid': actor['pid'] + 100, 'pgid': actor['pid'] + 100, 'started': 'current native start'}
            table[process['pid']] = process
        def launch(domain, label, timeout=10):
            return next(({'label': label, 'pid': process['pid']} for name, actor in actors.items()
                for process in table.values() if actor['label'] == label and process['command'] == actor['process']['command']), None)
        with patch.dict(services['recovery_actors'].__globals__, launch_observation=launch):
            current = services['recovery_actors'](root, 'gui/0', previous, selected, table)
            assert current['follower']['pid'] != previous['actors']['follower']['pid']
            extra = {**table, 999: {'pid': 999, 'command': '/unowned --owner ' + str(root / 'inbound')}}
            rejected(lambda: services['recovery_actors'](root, 'gui/0', previous, selected, extra), 'Manual/unowned')
            bad = {pid: {**process, 'command': '/unreviewed/native --owner ' + str(root / 'inbound')} for pid, process in table.items()}
            with patch.dict(services['recovery_actors'].__globals__, launch_observation=lambda *args: {'pid': next(iter(bad))}):
                rejected(lambda: services['recovery_actors'](root, 'gui/0', previous, selected, bad), 'authenticated native')
            with patch.dict(services['recovery_actors'].__globals__,
                            launch_observation=lambda domain, label, timeout=10: None if label == actors['alice']['label'] else launch(domain, label, timeout)):
                rejected(lambda: services['recovery_actors'](root, 'gui/0', previous, selected, table), 'Source actor is not live')
        store(root / 'follower/state.json', {'roots': {}, 'original-intent': 'follower', 'naturallyAdvanced': True})
        reconciliation_path = root / 'original-intent-reconciliation.json'
        reconciliation_pin = {'path': str(reconciliation_path), 'sha256': store(reconciliation_path, {
            'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'status': 'VERIFIED', 'unresolvedOriginals': []})}
        with patch.dict(services['recovery_unstarted'].__globals__, launch_observation=lambda *args: None):
            services['recovery_unstarted'](root, 'gui/0', 'hoodi-milestone-1', reconciliation_pin)
            for relative in ('campaigns/hoodi-milestone-1', 'app-messages/intents'):
                path = root / relative; path.mkdir(parents=True)
                rejected(lambda: services['recovery_unstarted'](root, 'gui/0', 'hoodi-milestone-1', reconciliation_pin), 'already')
                path.rmdir()
        # The same sealed verification initially pins recovery-1; later attempts bind the current hash without resealing.
        sealed_verification = {'continuationPredecessor': copy.deepcopy(previous_pin)}
        for number in (2, 4):
            pin = services['continuation_predecessor_pin'](root, sealed_verification, expected_sha=previous_pin['sha256'])
            _, current_pin = services['recovery_predecessor'](root, selected, pin)
            assert current_pin == previous_pin
            record, state = attempt('hoodi-milestone-1-recovery-' + str(number), current_pin, unapplied=number == 4)
            rejected(lambda: services['continuation_predecessor_pin'](root, sealed_verification, state, 'changed'),
                     'Resume predecessor digest changed')
            with patch.dict(services['recovery_predecessor'].__globals__, GEAR_SHA=manifest['files']['bin/gear']):
                services['validate_recovery_record'](root, state, record)
                if number == 4:
                    # Fresh preparations authenticate history before their gate clocks exist.
                    preparation = copy.deepcopy(state)
                    for field in ('preparedAtMs', 'lastWallMs', 'stopGate', 'sourceGate', 'progressGate', 'recoveryAuthorizationSha256'):
                        preparation.pop(field)
                    original_preparation = copy.deepcopy(preparation)
                    services['validate_recovery_history'](root, preparation, record)
                    assert preparation == original_preparation
                    history = next(iter(preparation['historyTrees']))
                    preparation['historyTrees'][history] = '00' * 32
                    rejected(lambda: services['validate_recovery_history'](root, preparation, record), 'history changed')
                    # Initial admission and resume retain installed recovery-2, not unbound recovery-3.
                    parked = root.parent / 'next-preparation'; record.parent.rename(parked)
                    try:
                        def admit():
                            return services['recovery_predecessor'](root, selected, current_pin, successor_name=state['transitionName'])
                        admitted, admitted_pin = admit()
                        assert admitted['transitionName'] == 'hoodi-milestone-1-recovery-2' and admitted_pin == current_pin
                        assert services['load'](root / 'supervisors/hoodi-milestone-1-observer-binding.json')['transition'] == current_pin['path']
                        extras = set(failed_state['mutableFiles']) - set(previous['mutableFiles'])
                        assert extras == {'supervisors/hoodi-milestone-1-retry-1-' + mode + '.plist' for mode in ('preflight', 'warmup')}
                        for relative in extras:
                            assert failed_state['mutableFiles'][relative]['old'] is None
                            partial = root / relative
                            store(partial, (failed_record.parent / 'after' / relative).read_bytes())
                            rejected(admit, 'partially applied'); partial.unlink()
                        old_plist = root / 'supervisors/hoodi-milestone-1-preflight.plist'
                        old_raw = old_plist.read_bytes(); store(old_plist, old_raw + b'\n')
                        rejected(admit, 'applied binding'); store(old_plist, old_raw)
                        saved = failed_record.read_bytes()
                        missing = root.parent / 'missing-failed-record'; failed_record.rename(missing)
                        rejected(admit, 'Missing continuation history'); missing.rename(failed_record)
                        extra = root / 'bundle-transitions/hoodi-milestone-1-recovery-6'; extra.mkdir()
                        rejected(admit, 'inventory'); extra.rmdir()
                        for field in ('runId', 'transitionName', 'newBundle', 'helperSha256', 'historyTrees',
                                      'status', 'phase', 'stopGate', 'sourceGate', 'progressGate', 'starts', 'snapshotTreeSha256'):
                            altered = copy.deepcopy(failed_state)
                            if field == 'newBundle': altered[field]['sha256'] = 'cd' * 32
                            elif field == 'historyTrees': altered[field].pop(next(iter(altered[field])))
                            elif field == 'stopGate': altered[field]['status'] = 'RUNNING'
                            elif field in ('sourceGate', 'progressGate'): altered[field]['status'] = 'PASS'
                            elif field == 'starts': altered[field]['alice'] = copy.deepcopy(actors['alice'])
                            elif field == 'status': altered[field] = 'RUNNING'
                            elif field == 'phase': altered[field] = 'descriptor-and-supervisors-bound'
                            else: altered[field] = 'changed'
                            store(failed_record, altered); rejected(admit); store(failed_record, saved)
                        for gate, deadline_field in (('stopGate', 'deadlineAtMs'), ('sourceGate', 'absoluteCapAtMs'), ('progressGate', 'deadlineAtMs')):
                            altered = copy.deepcopy(failed_state); altered[gate][deadline_field] += 1
                            store(failed_record, altered); rejected(admit, 'frozen'); store(failed_record, saved)
                        frozen = failed_record.parent / 'recovery-authorization.json'; frozen.chmod(0o600)
                        rejected(admit, 'Frozen'); frozen.chmod(0o400)
                        replacement = failed_record.parent / 'after/run.json'; raw = replacement.read_bytes()
                        replacement.chmod(0o600); store(replacement, raw + b'\n')
                        rejected(admit, 'replacement'); store(replacement, raw); replacement.chmod(0o400)
                        binding = root / 'supervisors/hoodi-milestone-1-observer-binding.json'; raw = binding.read_bytes()
                        store(binding, (failed_record.parent / 'after' / str(binding.relative_to(root))).read_bytes())
                        rejected(admit, 'applied binding'); store(binding, raw)
                        old_record = original.read_bytes(); store(original, old_record + b'\n')
                        rejected(admit, 'history'); store(original, old_record)
                        legacy_bytes = legacy.read_bytes(); store(legacy, legacy_bytes + b'\n')
                        rejected(admit, 'history'); store(legacy, legacy_bytes)
                        unrelated = root / 'bundle-transitions/unowned-transition'; unrelated.mkdir()
                        rejected(admit, 'history'); unrelated.rmdir()
                        assert failed_record.read_bytes() == saved
                        assert state['historyTrees'][str(failed_record.parent.relative_to(root))] == services['tree_digest'](failed_record.parent)
                    finally:
                        parked.rename(record.parent)
                    bind_attempt(record, state)
            for field in ('candidate', 'helper', 'actor', 'stop', 'source', 'progress', 'target'):
                altered = copy.deepcopy(state)
                if field == 'candidate': altered['newBundle']['sha256'] = 'substituted'
                elif field == 'helper': altered['helperSha256'] = 'substituted'
                elif field == 'actor': altered['actors']['alice']['pid'] += 1
                elif field == 'stop': altered['stopGate']['deadlineAtMs'] += 1
                elif field == 'source': altered['sourceGate']['absoluteCapAtMs'] += 1
                elif field == 'target': altered['rootScanTarget']['height'] += 1
                else: altered['progressGate']['deadlineAtMs'] += 1
                with patch.dict(services['recovery_predecessor'].__globals__, GEAR_SHA=manifest['files']['bin/gear']):
                    rejected(lambda: services['validate_recovery_record'](root, altered, record), 'frozen candidate/helper/actor/deadline')
            old = original.read_bytes(); store(original, old + b'\n')
            with patch.dict(services['recovery_predecessor'].__globals__, GEAR_SHA=manifest['files']['bin/gear']):
                rejected(lambda: services['validate_recovery_record'](root, state, record))
            store(original, old)
            rejected(lambda: admission['completed_transition'](root, config, bundle, manifest), 'bound transition has not passed')
            state['status'] = 'PASS'; state['progressGate']['status'] = 'FAILED'
            with patch.dict(services['recovery_predecessor'].__globals__, GEAR_SHA=manifest['files']['bin/gear']):
                rejected(lambda: services['validate_recovery_record'](root, state, record), 'Failed continuation gate')
            evidence = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'transitionName': state['transitionName'],
                'authorization': authorization_pin, 'scanTarget': state['rootScanTarget'], 'scanner': {'block': 45},
                'execution': {'queueBlock': 5}, 'allPublicationsFinalized': True, 'queueBlockDelta': 0,
                'criterion': 'verified-idle-root-continuity', 'roots': [{'sourceBlock': 5}]}
            proof = record.parent / 'queue-root-continuity.json'
            state['queueRootContinuity'] = {'path': str(proof), 'sha256': store(proof, evidence)}
            state['appliedObservation'] = {'queueRootEvidence': evidence}
            state['progressGate']['status'], state['phase'] = 'PASS', 'applied-progress-and-originals-verified'
            store(record, state)
            accepted, source = admission['completed_transition'](root, config, bundle, manifest)
            assert accepted['transitionName'] == state['transitionName'] and source['phase'] == 'ready'
            _, previous_pin = services['recovery_predecessor'](root, selected, {'path': str(record), 'sha256': services['digest'](record)})
            if number == 2:
                failed_record, failed_state = attempt('hoodi-milestone-1-recovery-3', previous_pin, failed=True,
                    unapplied=True, candidate_pin=retry_selected)
                failure_hashes[str(failed_record)] = services['digest'](failed_record)
        # Expiry before any launchctl call remains HOLD without resetting either later gate.
        expired = copy.deepcopy(failed_state); expired['status'] = expired['stopGate']['status'] = 'RUNNING'
        gates = copy.deepcopy({name: expired[name] for name in ('stopGate', 'sourceGate', 'progressGate')})
        def no_stop(*args, **kwargs):
            raise AssertionError('Expired preparation must not issue a supervisor command')
        stop_record = root / 'stop-check.json'
        with patch.dict(services['stop_barrier'].__globals__, launch_observation=no_stop):
            rejected(lambda: services['stop_barrier'](root, 'gui/0', expired, stop_record, expired['stopGate']), 'deadline expired')
            stopped = services['load'](stop_record)
            assert stopped['status'] == 'HOLD' and stopped['stopGate']['status'] == 'FAILED'
            assert stopped['stopGate']['deadlineAtMs'] == gates['stopGate']['deadlineAtMs']
            assert stopped['sourceGate'] == gates['sourceGate'] and stopped['progressGate'] == gates['progressGate'] and not stopped['starts']
            rejected(lambda: services['stop_barrier'](root, 'gui/0', stopped, stop_record, stopped['stopGate']), 'already failed')
            assert services['load'](stop_record)['stopGate']['deadlineAtMs'] == gates['stopGate']['deadlineAtMs']
        # A dispatched actor can restart while the parent is absent. Journal both HOLD and expiry.
        import argparse
        verification = {'campaignName': 'hoodi-milestone-1', 'continuationAuthorization': authorization_pin,
                        'continuationPredecessor': previous_pin, 'applicationArtifacts': {},
                        'originalIntentReconciliation': reconciliation_pin}
        store(root / 'forge-final/deployment.lock', b'')
        store(root / 'deployment.json', {'ethereum': {'chainId': 560048, 'queue': '0x' + '12' * 20}})
        store(root / 'token-stack/token-stack.json', {'checkpoint': 'checkpoint'})
        record, state = attempt('hoodi-milestone-1-recovery-5', previous_pin)
        state['starts'] = {name: {**copy.deepcopy(actor), 'status': 'authenticated',
            'atMs': state['preparedAtMs'], 'authenticatedAtMs': state['preparedAtMs']} for name, actor in actors.items()}
        store(record, state)
        original_starts = copy.deepcopy(state['starts'])
        deadline = state['progressGate']['deadlineAtMs']
        runner = runpy.run_path(str(bundle / 'ops/setup-services.py'))
        def observed_actor(domain, label, timeout=10):
            return next(({'label': label, 'pid': actor['pid'] + (100 if name == 'follower' else 0)}
                         for name, actor in actors.items() if actor['label'] == label), None)
        args = argparse.Namespace(candidate_bundle=bundle, candidate_sha256=selected['sha256'],
            transition_name=state['transitionName'], predecessor_sha256=previous_pin['sha256'], resume=True)
        with patch.dict(runner['transition'].__globals__, authenticate_bundle=lambda *args: manifest,
                        qualify_candidate=lambda *args: verification, bundle_diff=lambda *args: {},
                        qualify_application_rebind=lambda *args: None,
                        launch_observation=observed_actor, process_table=lambda *args: {actor['pid']: actor['process'] for actor in actors.values()}), \
             patch.object(runpy, 'run_path', return_value={'app_artifact': lambda *args: None,
                        'qualified_campaign': admission['qualified_campaign']}), patch.dict(os.environ, BEEFY_RUN=str(root)):
            for expired in (False, True):
                now = deadline + 1 if expired else state['preparedAtMs'] + 5000
                with patch.dict(runner['transition'].__globals__, clock_ms=lambda: now):
                    rejected(lambda: runner['transition'](args), 'deadline expired' if expired else 'identity changed')
                saved = services['load'](record)
                assert saved['status'] == 'HOLD'
                assert saved['progressGate']['status'] == ('FAILED' if expired else 'HOLD')
                assert saved['progressGate']['deadlineAtMs'] == deadline and saved['starts'] == original_starts
                assert services['load'](root / 'supervisors/bridge-services.json')['status'] == 'HOLD'
        assert all(services['digest'](Path(path)) == expected for path, expected in failure_hashes.items())
        assert services['digest'](root / 'source-chain/launch-state.json') == launch_sha
        for name in ('../recovery-2', 'hoodi-milestone-1-recovery-0', 'hoodi-milestone-1-recovery-02', 'other'):
            rejected(lambda: services['recovery_number'](name))
        assert services['recovery_number']('hoodi-milestone-1-recovery-29') == 29
        old = {'files': {'bin/gear': services['GEAR_SHA'], 'bin/beefy-relay': 'old', 'bin/relayer': 'unchanged',
                        'bin/checkpoints-tool': 'unchanged', 'ops/setup-services.py': 'old', 'verification.json': 'old'},
               'solidity': {}, 'binaries': {}}
        new = {**old, 'files': {**old['files'], 'bin/beefy-relay': 'new', 'ops/setup-services.py': 'reviewed', 'verification.json': 'fresh'}}
        assert 'bin/beefy-relay' in services['bundle_diff'](old, new, True)
        rejected(lambda: services['bundle_diff'](old, {**new, 'files': {**new['files'], 'bin/gear': 'changed'}}, True))
        proof = {'sourceFiles': {'source.rs': 'reviewed'}, 'checks': [{'name': name} for name in ('beefy-nextest', 'full-release-build', 'historical-recovery')],
            'followerCatchupFix': {'previousSha256': 'old', 'candidateSha256': 'wrong', 'sourceFiles': {'source.rs': 'reviewed'},
                                 'checks': ['beefy-nextest', 'full-release-build', 'historical-recovery']}}
        rejected(lambda: services['qualify_native_changes'](old, new, proof))
        proof['followerCatchupFix']['candidateSha256'] = 'new'
        inherited = {**proof, 'checks': proof['checks'] + [{'name': 'cargo-tests'}],
            'followerCatchupFix': {**proof['followerCatchupFix'],
            'checks': ['cargo-tests', 'full-release-build', 'historical-recovery']}}
        rejected(lambda: services['qualify_native_changes'](old, new, inherited))
        services['qualify_native_changes'](old, new, proof)
        relayer = {**old, 'files': {**old['files'], 'bin/relayer': 'new-relayer'}}
        assert services['bundle_diff'](old, relayer, True) == {'bin/relayer': {'old': 'unchanged', 'new': 'new-relayer'}}
        rejected(lambda: services['bundle_diff'](old, relayer))
        rejected(lambda: services['qualify_native_changes'](old, relayer, proof))
        review = {'previousSha256': 'unchanged', 'candidateSha256': 'new-relayer',
                  'sourceFiles': {'sender.rs': 'reviewed'},
                  'checks': ['cargo-tests', 'full-release-build', 'historical-recovery']}
        qualified = {'sourceFiles': review['sourceFiles'], 'checks': [{'name': name} for name in review['checks']],
                     'relayerSchedulingFix': review}
        services['qualify_native_changes'](old, relayer, qualified)
        for field, value in [('previousSha256', 'wrong'), ('candidateSha256', 'wrong'),
                             ('sourceFiles', {'sender.rs': 'unqualified'}), ('sourceFiles', {}),
                             ('checks', ['full-release-build', 'historical-recovery']),
                             ('checks', [*review['checks'], 'missing-check'])]:
            rejected(lambda field=field, value=value: services['qualify_native_changes'](
                old, relayer, {**qualified, 'relayerSchedulingFix': {**review, field: value}}))
        for field in ('bin/gear', 'bin/checkpoints-tool'):
            rejected(lambda field=field: services['bundle_diff'](
                old, {**relayer, 'files': {**relayer['files'], field: 'changed'}}, True))
    print('Repeatable continuation passed: installed baseline, authenticated unbound failures, immutable history/deadlines, fresh source ownership, HOLD stop failures and PASS-only admission')


def check_continuation_funding():
    import copy
    from eth_account import Account
    services = runpy.run_path(str(Path(__file__).with_name('setup-services.py')))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary); (root / 'hoodi').mkdir(); (root / 'follower').mkdir()
        signer = Account.from_key('0x' + '01' * 32)
        other = Account.from_key('0x' + '02' * 32)
        roles = {'follower': signer.address, **{role: '0x' + byte * 20 for role, byte in (('root', '12'), ('paid', '34'), ('campaign', '56'))}}
        (root / 'hoodi/addresses.json').write_text(json.dumps({'roles': roles}))
        ethereum = {'client': other.address.lower()}
        transactions = {}
        def signed_submission(nonce, account=signer, **changes):
            fields = {'chainId': 560048, 'nonce': nonce, 'to': other.address, 'value': 0,
                      'gas': 21000, 'maxFeePerGas': 20, 'maxPriorityFeePerGas': 1, 'type': 2, **changes}
            signed = account.sign_transaction(fields)
            tx_hash = '0x' + signed.hash.hex()
            transactions[tx_hash] = {'hash': tx_hash, 'from': account.address, 'to': fields['to'], 'nonce': hex(fields['nonce'])}
            return {'nonce': nonce, 'clientAddress': ethereum['client'], 'txHash': tx_hash,
                    'rawTransaction': '0x' + signed.raw_transaction.hex()}
        def commitment(submission):
            return {'clientAddress': ethereum['client'], 'txHash': submission['txHash'],
                    'finalized': False, 'submission': submission}
        first = signed_submission(861)
        follower = {'followerSigner': roles['follower'], 'rootPublisherSigner': roles['root'], 'activeEthereum': ethereum,
                    'submission': first, 'commitments': []}
        journal = root / 'follower/state.json'; journal.write_text(json.dumps(follower))
        nonces = {'finalized': 861, 'latest': 861, 'pending': 861}
        fault = None
        def rpc(config, method, params, timeout=10):
            if method == 'eth_getBalance': return hex(1 if fault == 'low-balance' else 10000000)
            if method == 'eth_getTransactionCount':
                return hex((862 if fault == 'consumed-nonce' else nonces['finalized' if isinstance(params[1], dict) else params[1]])
                           if params[0] == roles['follower'] else 0)
            if method == 'eth_getTransactionByHash':
                observed = transactions.get(params[0]) if nonces['pending'] > nonces['finalized'] else None
                return {**observed, 'nonce': hex(0)} if observed is not None and fault == 'substituted' else observed
            if method == 'eth_getTransactionReceipt':
                observed = transactions.get(params[0])
                return {'transactionHash': params[0], 'status': '0x1'} if observed and int(observed['nonce'], 16) < nonces['latest'] else None
            raise AssertionError(method)
        budget = {'minimumBalanceWei': {role: '500000' for role in roles}}
        execution = {'finalizedHash': '0x' + 'ef' * 32}
        def account():
            before = {path.relative_to(root): path.read_bytes() for path in root.rglob('*') if path.is_file()}
            try:
                return services['continuation_funding'](root, {}, {'ethereum': ethereum}, budget)
            finally:
                assert {path.relative_to(root): path.read_bytes() for path in root.rglob('*') if path.is_file()} == before
        with patch.dict(services['continuation_funding'].__globals__, execution_state=lambda *args: execution, execution_rpc=rpc):
            evidence = account()
            assert evidence['originalFollowerSubmission']['nonce'] == '861'
            assert evidence['originalFollowerSubmission']['maximumCostWei'] == '420000'
            assert 'rawTransaction' not in json.dumps(evidence) and evidence['fundingPerformedByTransition'] is False
            for failure in ('low-balance', 'consumed-nonce'):
                fault = failure
                rejected(account)
            fault = None
            altered = copy.deepcopy(follower); altered['submission']['nonce'] = 862
            journal.write_text(json.dumps(altered)); rejected(account)

            mined = {**follower, 'submission': None, 'commitments': [commitment(first), commitment(signed_submission(862))]}
            journal.write_text(json.dumps(mined)); nonces.update(latest=863, pending=863)
            evidence = account()
            assert evidence['originalFollowerSubmission'] is None
            assert [item['nonce'] for item in evidence['unsettledFollowerSubmissions']] == ['861', '862']
            assert all(item['transactionObserved'] and item['receiptObserved'] for item in evidence['unsettledFollowerSubmissions'])
            assert 'rawTransaction' not in json.dumps(evidence)
            active = {**mined, 'submission': signed_submission(863)}
            journal.write_text(json.dumps(active)); nonces['pending'] = 864
            evidence = account()
            assert evidence['originalFollowerSubmission']['nonce'] == '863'
            assert evidence['originalFollowerSubmission']['transactionObserved'] and not evidence['originalFollowerSubmission']['receiptObserved']
            assert [item['nonce'] for item in evidence['unsettledFollowerSubmissions']] == ['861', '862']
            journal.write_text(json.dumps(mined)); rejected(account)
            nonces['pending'] = 863
            for failure in ('missing', 'missing-signed', 'tampered', 'nonce', 'conflict', 'binding'):
                altered = copy.deepcopy(mined)
                if failure == 'missing': altered['commitments'].pop()
                elif failure == 'missing-signed': altered['commitments'][1]['submission'] = None
                elif failure == 'tampered': altered['commitments'][1]['submission']['rawTransaction'] = signed_submission(862, data='0x01')['rawTransaction']
                elif failure == 'nonce': altered['commitments'][1]['submission']['nonce'] = 860
                elif failure == 'conflict': altered['commitments'].append(commitment(signed_submission(861, data='0x01')))
                else: altered['commitments'][1]['txHash'] = first['txHash']
                journal.write_text(json.dumps(altered)); rejected(account)
            for changes, sender in (({'chainId': 560049}, signer), ({'to': roles['root']}, signer),
                                    ({'value': 1}, signer), ({}, other), ({'maxFeePerGas': 1000000}, signer)):
                altered = copy.deepcopy(mined)
                altered['commitments'][1] = commitment(signed_submission(862, account=sender, **changes))
                journal.write_text(json.dumps(altered)); rejected(account)
            journal.write_text(json.dumps(mined)); fault = 'substituted'; rejected(account); fault = None
            nonces['pending'] = (1 << 64) - 1
            rejected(account)
    print('Continuation funding passed: signed ownership of every unsettled nonce, original active intent, funding and no-write checks')


def check_verified_idle_roots():
    import copy
    import types
    import substrateinterface
    from eth_abi import encode
    from eth_utils import keccak, to_checksum_address
    services = runpy.run_path(str(Path(__file__).with_name('setup-services.py')))
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary).resolve() / services['RETAINED_RUN']; root.mkdir()
        def store(path, value):
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(services['json_bytes'](value)); path.chmod(0o600)
        def source_hash(block): return '0x' + block.to_bytes(32, 'big').hex()
        def destination_hash(block): return '0x' + (block + 1000).to_bytes(32, 'big').hex()
        publisher, queue, client = '0x' + 'ab' * 20, '0x' + '12' * 20, '0x' + '34' * 20
        ethereum = {'sourceDomain': '0x' + '56' * 32, 'bridgeDomain': '0x' + '78' * 32, 'queue': queue, 'client': client}
        deployment = {'anchor': {'sourceGenesis': source_hash(0)}, 'ethereum': ethereum}
        config = {'source': {'aliceRpc': 'ws://source', 'bobRpc': 'ws://witness'},
                  'network': {'executionHttp': 'https://owned-fixture', 'genesisHash': destination_hash(0)}}
        store(root / 'hoodi/addresses.json', {'roles': {'root': to_checksum_address(publisher)}})
        store(root / 'run.json', {'funding': {'address': publisher}})
        idle_path = root / 'continuation-authorization.json'
        store(idle_path, {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'distributionAddress': publisher,
                         'routineHoodiFixesRestartsDeploymentsAndSeparateTestAttemptsAuthorized': True,
                         'refillAndDistributionAuthorized': True, 'failedHistoryPreserved': True,
                         **{name: False for name in ('resetFailedDeadlines', 'replaceUnresolvedSignedIntents',
                                                    'mainnetSigningAuthorized', 'mainnetActivationAuthorized')}})
        authorization = {'path': str(idle_path), 'sha256': services['digest'](idle_path)}
        predecessor = root / 'bundle-transitions/hoodi-milestone-1-recovery-2/transition.json'
        roots, publications, transactions, receipts = {}, {}, {}, {}
        def registration(block, nonce):
            value = '0x' + keccak(text='owned-root-' + str(block)).hex()
            key = str(block) + '-' + value[2:]
            path = root / 'follower/root-publications' / (key + '.json')
            item = {'block': block, 'blockHash': source_hash(block), 'kind': 'merkleRoot', 'queueId': 0,
                    'queueRoot': value, 'status': 'accepted', 'publication': str(path)}
            raw = bytes([1, 2, block]); tx_hash = '0x' + keccak(raw).hex()
            proof = b'owned-proof'
            inclusion = 50 + nonce
            publication = {'schemaVersion': 3, 'status': 'accepted', 'finalityStatus': 'finalized', 'kind': 'merkleRoot',
                           'sourceBlock': block, 'root': value, 'sender': publisher, 'nonce': str(nonce),
                           'acceptedAnchorClient': client, 'rawTransaction': '0x' + raw.hex(), 'txHash': tx_hash,
                           'queueProof': '0x' + proof.hex(),
                           'proof': {'sourceBlock': block, 'sourceHash': source_hash(block), 'queueRoot': value, 'queueId': 0},
                           'sourceIdentity': {'sourceGenesis': source_hash(0), 'sourceDomain': ethereum['sourceDomain'],
                                              'bridgeDomain': ethereum['bridgeDomain'], 'destinationChainId': 560048, 'destinationQueue': queue},
                           'publicationReceipt': {'block': inclusion, 'blockHash': destination_hash(inclusion), 'finalized': True,
                                                  'finalizedBlock': 60, 'finalizedBlockHash': destination_hash(60)}}
            calldata = keccak(text='submitMerkleRoot(uint256,bytes32,bytes)')[:4] + encode(['uint256', 'bytes32', 'bytes'], [block, bytes.fromhex(value[2:]), proof])
            transactions[tx_hash] = {'hash': tx_hash, 'from': publisher, 'to': queue, 'nonce': hex(nonce), 'value': '0x0',
                                     'input': '0x' + calldata.hex(), 'blockNumber': hex(inclusion), 'blockHash': destination_hash(inclusion)}
            log = {'address': queue, 'topics': ['0x' + keccak(text='MerkleRoot(uint256,bytes32)').hex()],
                   'data': '0x' + encode(['uint256', 'bytes32'], [block, bytes.fromhex(value[2:])]).hex(), 'removed': False,
                   'transactionHash': tx_hash, 'blockHash': destination_hash(inclusion), 'blockNumber': hex(inclusion), 'logIndex': '0x0'}
            receipts[tx_hash] = {'transactionHash': tx_hash, 'from': publisher, 'to': queue, 'status': '0x1',
                                 'blockNumber': hex(inclusion), 'blockHash': destination_hash(inclusion), 'logs': [log]}
            roots[key], publications[key] = item, publication
            store(path, publication)
            return key
        original_key = registration(5, 0)
        store(predecessor.parent / 'before-runtime/follower/state.json', {'roots': copy.deepcopy(roots)})
        store(predecessor.parent / 'before-runtime/follower/root-publications' / (original_key + '.json'), publications[original_key])
        follower = {'rootPublisherSigner': publisher, 'rootScan': {'block': 46, 'blockHash': source_hash(46)},
                    'follower': {'status': 'healthy', 'lastFinalizedUpdate': 48}, 'roots': roots}
        state = {'transitionName': 'hoodi-milestone-1-recovery-2', 'continuationAuthorization': authorization,
                 'rootScanTarget': {'height': 45, 'hash': source_hash(45)}, 'predecessor': {'path': str(predecessor)},
                 'executionBefore': {'queueBlock': 5, 'beefyBlock': 7}, 'checkpointBefore': {'slot': 10}}
        execution = {'queueBlock': 5, 'beefyBlock': 48, 'finalizedHeight': 100, 'finalizedHash': destination_hash(100)}
        checkpoint = {'slot': 11, 'replaying': False}
        fault = None
        class Source:
            def __init__(self, url, **kwargs): self.url = url
            def get_block_hash(self, block):
                if fault == 'cursor' and block == 46: return source_hash(47)
                if fault == 'target' and block == 45: return source_hash(47)
                return source_hash(block)
            def get_chain_finalised_head(self): return source_hash(100)
            def get_block_number(self, block): return 100
            def query(self, module, field, block_hash):
                block = int(block_hash, 16)
                expected = next(item for item in roots.values() if item['block'] == block)
                value = expected['queueId'] if field == 'QueueId' else expected['queueRoot']
                if fault == 'witness' and self.url == config['source']['bobRpc'] and field == 'QueueMerkleRoot': value = source_hash(99)
                return types.SimpleNamespace(value=value)
            def get_events(self, block_hash):
                block = int(block_hash, 16); expected = next(item for item in roots.values() if item['block'] == block)
                attributes = {'queue_id': expected['queueId'], 'root': expected['queueRoot']}
                if fault == 'source-event': attributes['root'] = source_hash(99)
                return [types.SimpleNamespace(value={'event': {'module_id': 'GearEthBridge', 'event_id': 'QueueMerkleRootChanged', 'attributes': attributes}})]
            def close(self): pass
        def rpc(unused, method, params, timeout=10):
            if method == 'eth_chainId': return hex(560048)
            if method == 'eth_getBlockByNumber':
                number = 100 if params[0] == 'finalized' else int(params[0], 16)
                return {'number': hex(number), 'hash': destination_hash(number)}
            if method == 'eth_getTransactionCount': return hex(len(roots) + (1 if fault == 'pending-nonce' and params[1] == 'pending' else 0))
            if method == 'eth_getTransactionByHash':
                value = copy.deepcopy(transactions[params[0]])
                if fault == 'calldata': value['input'] = value['input'][:-2] + 'ff'
                return value
            if method == 'eth_getTransactionReceipt':
                value = copy.deepcopy(receipts[params[0]])
                if fault == 'failed': value['status'] = '0x0'
                if fault == 'nonfinal': value['blockNumber'] = hex(101)
                if fault == 'orphaned': value['blockHash'] = destination_hash(99)
                if fault == 'wrong-queue': value['logs'][0]['address'] = client
                if fault == 'ambiguous-log': value['logs'].append(copy.deepcopy(value['logs'][0]))
                return value
            if method == 'eth_call':
                block = int(params[0]['data'][-64:], 16)
                value = next(item['queueRoot'] for item in roots.values() if item['block'] == block)
                return source_hash(99) if fault == 'stored-root' else value
            raise AssertionError('Unexpected RPC ' + method)
        with patch.dict(services['recovery_root_continuity'].__globals__, execution_rpc=rpc), \
                patch.object(substrateinterface, 'SubstrateInterface', Source):
            ready = services['startup_progress_ready']
            assert ready(state, execution, checkpoint, follower), 'Verified idle roots must not deadlock recovery startup'
            assert not ready({**state, 'transitionName': services['TRANSITION']}, execution, checkpoint, follower)
            assert not ready(state, {**execution, 'beefyBlock': 7}, checkpoint, follower)
            assert not ready(state, {**execution, 'beefyBlock': 44}, checkpoint, follower), 'Advancement without finalized catch-up is not ready'
            assert not ready(state, execution, checkpoint, {**follower, 'follower': {'status': 'healthy', 'lastFinalizedUpdate': 44}})
            assert not ready(state, execution, {**checkpoint, 'slot': 10}, follower)
            assert not ready(state, execution, {**checkpoint, 'replaying': True}, follower)
            assert not ready(state, execution, checkpoint, {**follower, 'follower': {'status': 'failed'}})
            assert not ready(state, {**execution, 'queueBlock': 4}, checkpoint, follower)
            stale = {**follower, 'rootScan': {'block': 44, 'blockHash': source_hash(44)}}
            assert not ready(state, execution, checkpoint, stale)
            rejected(lambda: services['recovery_root_continuity'](root, config, deployment, state, execution, stale), 'pinned finalized target')
            proof = services['recovery_root_continuity'](root, config, deployment, state, execution, follower)
            assert proof['criterion'] == 'verified-idle-root-continuity' and proof['queueBlockDelta'] == 0
            assert proof['roots'][0]['sourceBlock'] == 5 and proof['roots'][0]['root'] == roots[original_key]['queueRoot']
            for failure in ('cursor', 'target', 'witness', 'source-event', 'pending-nonce', 'calldata', 'failed', 'nonfinal', 'orphaned', 'wrong-queue', 'ambiguous-log', 'stored-root'):
                fault = failure
                rejected(lambda: services['recovery_root_continuity'](root, config, deployment, state, execution, follower))
            fault = None
            orphan = root / 'follower/root-publications/orphan.json.tmp'; store(orphan, {'status': 'pending'})
            rejected(lambda: services['recovery_root_continuity'](root, config, deployment, state, execution, follower), 'Orphan or unfinished')
            orphan.unlink()
            rejected(lambda: services['recovery_root_continuity'](root, config, deployment, state, {**execution, 'queueBlock': 6}, follower), 'inventory regressed or is incomplete')
            new_key = registration(6, 1); execution['queueBlock'] = 6
            assert ready({**state, 'transitionName': services['TRANSITION']}, execution, checkpoint, follower)
            advanced = services['recovery_root_continuity'](root, config, deployment, state, execution, follower)
            assert advanced['criterion'] == 'finalized-new-root' and advanced['queueBlockDelta'] == 1
            for status in ('pending', 'mined', 'failed'):
                altered = copy.deepcopy(publications[new_key]); altered['status'] = status
                store(Path(roots[new_key]['publication']), altered)
                rejected(lambda: services['recovery_root_continuity'](root, config, deployment, state, execution, follower), 'not finalized')
            store(Path(roots[new_key]['publication']), publications[new_key])
    print('Verified idle startup passed: strict checkpoint/BEEFY/health gates, immutable scan target, original finalized roots, nonce/orphan/storage/receipt rejection')


def check():
    original_run = os.environ.get("BEEFY_RUN")
    try:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            sealer = runpy.run_path(str(Path(__file__).with_name("seal-artifacts.py")))
            source, copied = base / "qualified-source", base / "copied-artifact"
            source.write_bytes(b"qualified bytes")
            expected = hashlib.sha256(source.read_bytes()).hexdigest()
            sealer["copy_verified"](source, copied, expected)
            assert copied.read_bytes() == b"qualified bytes"
            original_copy = shutil.copyfile

            def replace_during_copy(src, dst):
                source.write_bytes(b"unqualified replacement")
                return original_copy(src, dst)

            with patch.object(shutil, "copyfile", side_effect=replace_during_copy):
                try:
                    sealer["copy_verified"](source, copied, expected)
                except SystemExit:
                    pass
                else:
                    raise AssertionError("Qualification-to-copy race was accepted")
            assert copied.read_bytes() == b"unqualified replacement"
            bundle, run = base / "bundle", base / "run"
            (bundle / "ops").mkdir(parents=True)
            (bundle / "bin").mkdir()
            run.mkdir()
            module = bundle / "ops/run_context.py"
            module.write_bytes(Path(__file__).with_name("run_context.py").read_bytes())
            deployment = bundle / "ops/prepare-token-deployment.py"
            deployment.write_bytes(Path(__file__).with_name("prepare-token-deployment.py").read_bytes())
            names = ("gear", "beefy-relay", "relayer", "checkpoints-tool")
            for name in names:
                # Artifact bytes exercise identity checks; these files are never executed.
                (bundle / "bin" / name).write_bytes(b"identity-test artifact: " + name.encode())
            project = run / "forge-final"
            solidity_snapshot(bundle, project)
            files = {str(path.relative_to(bundle)): hashlib.sha256(path.read_bytes()).hexdigest()
                     for path in bundle.rglob("*") if path.is_file()}
            manifest = bundle / "bundle.json"
            manifest.write_text(json.dumps({"schemaVersion": 1, "testOnly": True, "files": files,
                                           "solidity": {"scriptSha256": files["ethereum/script/BeefyTokens.s.sol"],
                                                        "artifactSha256": files["ethereum/out/BeefyTokens.s.sol/BeefyTokens.json"]},
                                           "binaries": {name: "bin/" + name for name in names}}))
            config = {"schemaVersion": 1, "runId": "run", "testOnly": True,
                      "bundle": {"path": str(bundle), "sha256": hashlib.sha256(manifest.read_bytes()).hexdigest()},
                      "network": {"chainId": 560048,
                                  "genesisHash": "0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b",
                                  "executionHttp": "https://invalid.example", "executionWss": "wss://invalid.example",
                                  "beaconHttp": "https://invalid.example"},
                      "source": {"aliceRpc": "ws://127.0.0.1:9962", "bobRpc": "ws://127.0.0.1:9963"},
                      "funding": {"wallet": str(base / "unused-wallet"), "address": "unused; never loaded"}}
            (run / "run.json").write_text(json.dumps(config))
            os.environ["BEEFY_RUN"] = str(run)
            context = runpy.run_path(str(module))
            key = base / "credential"
            key.write_text("private-test-value\n")
            key.chmod(0o600)
            assert context["private_text"](key) == "private-test-value"
            key.chmod(0o644)
            rejected(lambda: context["private_text"](key))
            key.chmod(0o600)
            link = base / "credential-link"
            link.symlink_to(key)
            rejected(lambda: context["private_text"](link))
            key.write_text(" " * 16385)
            rejected(lambda: context["private_text"](key))
            previous_context = sys.modules.pop("run_context", None)
            sys.path.insert(0, str(bundle / "ops"))
            try:
                deploy = runpy.run_path(str(deployment))
                check_deployment_inputs(deploy, project, bundle)
                check_simulation_binding(deploy, project, files)
                secret = "private-test-value"
                child = [sys.executable, "-c", "import sys; print(sys.argv[1]); print(sys.argv[1].upper()); print('public diagnostic'); sys.exit(int(sys.argv[2]))", secret, "0"]
                log = project / "check.log"
                with (project / "check.lock").open("a") as lock:
                    deploy["run_forge"](child, {}, log, (secret, ""), lock.fileno(), exclusive=True)
                    text = log.read_text()
                    assert secret not in text and secret.upper() not in text and "public diagnostic" in text
                    assert log.stat().st_mode & 0o777 == 0o600
                    try:
                        deploy["run_forge"](child, {}, log, (secret,), lock.fileno(), exclusive=True)
                    except FileExistsError:
                        pass
                    else:
                        raise AssertionError("Existing broadcast log was overwritten")
                    assert log.read_text() == text
                    child[-1] = "9"
                    rejected(lambda: deploy["run_forge"](child, {}, project / "failed.log", (secret,), lock.fileno(), exclusive=True))
            finally:
                sys.path.pop(0)
                sys.modules.pop("run_context", None)
                if previous_context is not None:
                    sys.modules["run_context"] = previous_context
            original = (bundle / "bin/relayer").read_bytes()
            (bundle / "bin/relayer").write_bytes(original + b"changed")
            rejected(lambda: context["artifact"]("relayer"))
            (bundle / "bin/relayer").write_bytes(original)
            config["source"]["aliceRpc"] = "ws://0.0.0.0:9962"
            (run / "run.json").write_text(json.dumps(config))
            rejected(lambda: runpy.run_path(str(module)))
            config["source"]["aliceRpc"] = "ws://127.0.0.1:9962"
            config["bundle"]["sha256"] = "0" * 64
            (run / "run.json").write_text(json.dumps(config))
            rejected(lambda: runpy.run_path(str(module)))
    finally:
        if original_run is None:
            os.environ.pop("BEEFY_RUN", None)
        else:
            os.environ["BEEFY_RUN"] = original_run
    check_checkpoint_trust_inputs()
    check_normal_runtime_admission()
    check_profiled_application_qualification()
    check_normal_activation_authorities()
    check_bootstrap_source_inventory()
    check_transition_review()
    check_milestone_qualification()
    check_milestone_ops()
    check_retry_campaign_admission()
    check_repeatable_continuation()
    check_continuation_funding()
    check_verified_idle_roots()
    print("Operational boundaries passed: private files, artifact identity, loopback source, descriptor hash")


if __name__ == "__main__":
    check()
