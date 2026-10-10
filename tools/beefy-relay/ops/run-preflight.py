#!/usr/bin/env python3
"""Admit one named bounded campaign or immutable app client; never start hourly qualification."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import signal
import stat
import subprocess
import time

APPLICATION_CHECK_COMMANDS = {
    'cargo build --locked -p ping --release',
    'forge build --root js/bridge-js/js-test/contracts --force --no-cache',
    'forge test --root js/bridge-js/js-test/contracts --match-contract MessageHandlerTest -vvv',
    'yarn workspace @gear-js/bridge typecheck',
    'yarn workspace @gear-js/bridge test test/vara-to-eth.test.ts',
    'yarn workspace @gear-js/bridge test test/eth-to-vara.test.ts',
    'yarn workspace @gear-js/bridge build:examples',
}

def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def contained(root, relative):
    root = Path(root)
    relative = Path(relative)
    require(not relative.is_absolute() and '..' not in relative.parts, 'Unsafe run-relative path')
    path = root
    for part in relative.parts:
        path = path / part
        require(not path.is_symlink(), 'Symlink in admission path: ' + str(relative))
    require(path.resolve().is_relative_to(root.resolve()), 'Path escapes the retained run')
    return path


def qualified_campaign(bundle, verification_sha256):
    raw = (bundle / 'verification.json').read_bytes()
    require(hashlib.sha256(raw).hexdigest() == verification_sha256, 'Selected campaign qualification changed')
    return campaign_name(json.loads(raw))


def campaign_name(selected):
    name = selected.get('campaignName')
    profile = selected.get('runtimeProfile')
    if profile is None:
        pattern = r'hoodi-milestone-1(?:-retry-[1-9][0-9]*)?'
    else:
        require(isinstance(profile, dict) and profile.get('name') in ('normal-runtime-hoodi', 'fast-runtime-hoodi'),
                'Unsupported qualified campaign runtime')
        prefix = 'hoodi-fast-runtime-' if profile['name'] == 'fast-runtime-hoodi' else 'hoodi-normal-runtime-'
        pattern = prefix + r'[a-z0-9][a-z0-9-]{0,42}'
    require(isinstance(name, str) and len(name) <= 64 and re.fullmatch(pattern, name) is not None,
            'Invalid qualified Hoodi campaign name')
    return name


def observer_binding_path(root, config):
    return contained(root, 'supervisors/normal-campaign-admission.json' if config.get('runtimeProfile') is not None
                     else 'supervisors/hoodi-milestone-1-observer-binding.json')



def campaign_directory(root, name, mode):
    require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,63}', name) is not None, 'Invalid campaign name')
    campaign = contained(root, 'campaigns/' + name)
    if campaign.exists():
        require(stat.S_ISDIR(campaign.lstat().st_mode) and stat.S_IMODE(campaign.stat().st_mode) == 0o700,
                'Campaign directory must be private mode 0700')
    journal = contained(root, 'campaigns/' + name + '/campaign-state.json')
    if journal.exists():
        require(stat.S_ISREG(journal.lstat().st_mode) and stat.S_IMODE(journal.stat().st_mode) == 0o600,
                'Campaign journal must be a regular mode-0600 file')
        value = json.loads(journal.read_text())
        require(value.get('schemaVersion') == 3 and value.get('runId') == name,
                'Campaign journal identity mismatch; preserve the original')
        if mode == 'preflight':
            require(value['preflight']['status'] in ('pending', 'running'), 'Existing preflight is terminal; no rerun')
        else:
            require(value['preflight']['status'] == 'passed', 'Native preflight has not passed')
        if mode == 'warmup':
            require(value['warmup']['status'] == 'pending', 'Existing warmup is not a fresh attempt; no rerun')
        if mode == 'app':
            require(value['warmup']['status'] == 'passed' and value['status'] == 'warmup_passed',
                    'Native warmup has not passed')
    else:
        require(mode == 'preflight', 'Named native preflight journal is missing')
        require(not campaign.exists() or not any(campaign.iterdir()), 'Fresh preflight directory is not empty')
    return campaign


def campaign_lock(root):
    path = contained(root, 'bounded-campaign.lock')
    inherited = os.environ.get('BEEFY_CAMPAIGN_LOCK_FD')
    if inherited is not None:
        require(re.fullmatch(r'[0-9]+', inherited) is not None, 'Invalid inherited campaign lock descriptor')
        fd = int(inherited)
        actual, expected = os.fstat(fd), path.lstat()
        require(stat.S_ISREG(actual.st_mode) and (actual.st_dev, actual.st_ino) == (expected.st_dev, expected.st_ino),
                'Inherited descriptor is not the deployment campaign lock')
        handle = os.fdopen(os.dup(fd), 'a')
        # A separate open must fail: an inherited but unlocked descriptor is not ownership.
        probe = os.open(path, os.O_RDWR | os.O_NOFOLLOW)
        try:
            try:
                fcntl.flock(probe, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except BlockingIOError:
                pass
            else:
                raise RuntimeError('Inherited campaign lock is not held')
        finally:
            os.close(probe)
    else:
        fd = os.open(path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        handle = os.fdopen(fd, 'a')
    require(stat.S_IMODE(os.fstat(handle.fileno()).st_mode) == 0o600, 'Campaign lock must be private')
    fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
    return handle


def validate_journal(root, campaign, config, launch):
    """Admission pins only; native Journal::validate_identity remains the full authoritative check."""
    from eth_utils import keccak
    path = campaign / 'campaign-state.json'
    if not path.exists():
        return
    value = json.loads(path.read_text())

    def native_digest(item):
        return '0x' + keccak(json.dumps(item, sort_keys=True, separators=(',', ':'), ensure_ascii=False).encode()).hex()

    deployment = json.loads((root / 'deployment.json').read_text())
    stack = json.loads((root / 'token-stack/token-stack.json').read_text())
    expected = {'deploymentManifestDigest': native_digest(deployment), 'tokenStackDigest': native_digest(stack),
                'sourceLaunchDigest': native_digest(launch['identity']), 'sourceGenesis': deployment['anchor']['sourceGenesis'],
                'bridgeDomain': deployment['anchor']['bridgeDomain'], 'rawSpecSha256': launch['identity']['rawSpecSha256']}
    expected['endpoints'] = {key + 'Digest': '0x' + keccak(text=endpoint).hex() for key, endpoint in
                             [('sourceRpc', config['source']['aliceRpc']), ('witnessRpc', config['source']['bobRpc']),
                              ('ethereumRpc', config['network']['executionWss'])]}
    expected['endpoints'].update(inboundDirectory=str(root / 'inbound'), outboundDirectory=str(root / 'outbound/journal'))
    evm = json.loads((root / 'hoodi/addresses.json').read_text())['roles']
    gear = json.loads((root / 'hoodi/gear-addresses.json').read_text())['roles']
    accounts = value['accounts']
    for field, address in [('campaignEvm', evm['campaign']), ('followerEvm', evm['follower']),
                           ('rootPublisherEvm', evm['root']), ('campaignGear', gear['campaign']['publicKey']),
                           ('governanceGear', gear['governance']['publicKey'])]:
        require(accounts.get(field, '').lower() == address.lower(), 'Campaign account identity changed')
    require(all(value.get(key) == item for key, item in expected.items()), 'Campaign lane identity changed before credentials')
    require(value.get('lastWallMs') is None or int(time.time() * 1000) >= value['lastWallMs'], 'Campaign clock regressed')


def app_arguments(flags):
    allowed = {'mode', 'intent-id', 'source-intent', 'application-id', 'payload-hex', 'resume', 'case'}
    values = {}
    for flag in flags:
        require(flag.startswith('--'), 'App flags must be keyed; no secret or positional arguments')
        key, equal, value = flag[2:].partition('=')
        require(key in allowed and key not in values, 'Unknown or duplicate app flag')
        require((key == 'resume' and not equal) or (key != 'resume' and equal and value), 'Malformed app flag')
        values[key] = value
    require(values.get('mode') in ('deploy', 'send', 'relay', 'verify', 'probe'), 'Explicit app mode required')
    require('intent-id' in values, 'App intent ID required')
    return values


def app_artifact(root, name, direction, bundle, verification_sha256, *, qualification=None):
    verification_bytes = (bundle / 'verification.json').read_bytes()
    require(hashlib.sha256(verification_bytes).hexdigest() == verification_sha256, 'Selected application qualification changed')
    selected = json.loads(verification_bytes)
    require(campaign_name(selected) == name, 'Application campaign differs from component qualification')
    if selected.get('runtimeProfile') is not None:
        require('applicationArtifacts' not in selected, 'Profiled application artifacts must be qualified after deployment')
        if qualification is None:
            path = contained(root, 'qualification/' + name + '-applications.json')
            require(path.is_file() and not path.stat().st_mode & 0o222, 'Immutable post-deployment application qualification is missing')
            qualification = json.loads(path.read_bytes())
        require(qualification.get('schemaVersion') == 1 and qualification.get('testOnly') is True
                and qualification.get('status') == 'VERIFIED' and qualification.get('runId') == root.name
                and qualification.get('campaignName') == name and qualification.get('runtimeProfile') == selected['runtimeProfile']
                and qualification.get('componentBundleSha256') == hashlib.sha256((bundle / 'bundle.json').read_bytes()).hexdigest()
                and qualification.get('componentVerificationSha256') == verification_sha256,
                'Post-deployment application qualification differs from the original lane')
        checks = qualification['checks']
        require(APPLICATION_CHECK_COMMANDS <= {check['command'] for check in checks}, 'Full owned application checks are missing')
        for check in checks:
            require(check['exitCode'] == 0 and check['command'], 'Application qualification includes a failed check')
            requested_log = Path(check['logPath'])
            require(requested_log.is_absolute() and requested_log.is_relative_to(root / 'qualification'), 'Application check log escapes its owner')
            log = contained(root, requested_log.relative_to(root))
            require(log.is_file() and not log.stat().st_mode & 0o222 and hashlib.sha256(log.read_bytes()).hexdigest() == check['logSha256'],
                    'Immutable application check evidence changed')
        binding = qualification['applicationArtifacts']
    else:
        require(qualification is None, 'Retained application qualification remains sealed')
        binding = selected['applicationArtifacts']
    requested = Path(binding['path'])
    owner = contained(root, 'app-messages')
    require(requested.is_absolute() and requested != owner and requested.is_relative_to(owner),
            'App artifact path escapes its application owner')
    directory = contained(root, requested.relative_to(root))
    require(requested == directory.resolve(strict=True), 'App artifact path is not canonical')
    require(stat.S_ISDIR(directory.lstat().st_mode) and not directory.stat().st_mode & 0o222,
            'App artifact directory is not immutable')
    manifest_path = contained(directory, 'manifest.json')
    require(stat.S_ISREG(manifest_path.lstat().st_mode) and not manifest_path.stat().st_mode & 0o222,
            'App artifact manifest is not immutable')
    manifest_bytes = manifest_path.read_bytes()
    require(binding['path'] == str(directory) and hashlib.sha256(manifest_bytes).hexdigest() == binding['manifestSha256'],
            'App artifact manifest differs from selected qualification')
    manifest = json.loads(manifest_bytes)
    require(manifest.get('schemaVersion') == 1 and manifest.get('testOnly') is True
            and manifest.get('runId') == root.name and manifest.get('campaignName') == name, 'App artifact identity mismatch')
    files = manifest.get('files')
    require(isinstance(files, dict) and files, 'App artifact file pins are missing')
    actual = set()
    for path in directory.rglob('*'):
        mode = path.lstat().st_mode
        require(stat.S_ISREG(mode) or stat.S_ISDIR(mode), 'Symlink/special entry in app artifacts')
        require(not mode & 0o222, 'App artifact tree is not immutable')
        if stat.S_ISREG(mode) and path != manifest_path:
            actual.add(str(path.relative_to(directory)))
    require(actual == set(files), 'App artifact inventory changed')
    for relative, expected in files.items():
        path = contained(directory, relative)
        require(hashlib.sha256(path.read_bytes()).hexdigest() == expected, 'App artifact changed: ' + relative)
    entry = 'lib/demo/example/' + direction + '.js'
    require(entry in files, 'Compiled application entry point is not pinned')
    require({'bin/node', 'lib/demo/example/eth-to-vara.js', 'lib/demo/example/vara-to-eth.js'} <= set(files)
            and os.access(directory / 'bin/node', os.X_OK), 'Pinned Node executable or compiled app entry point is missing')
    return directory / 'bin/node', directory / entry


def run_child(args, root, environment, lock, log_path, secrets=()):
    previous = {}
    log_path = contained(root, Path(log_path).relative_to(root))
    fd = os.open(log_path, os.O_WRONLY | os.O_APPEND | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    with os.fdopen(fd, 'a') as log:
        metadata = os.fstat(log.fileno())
        require(stat.S_ISREG(metadata.st_mode) and stat.S_IMODE(metadata.st_mode) == 0o600, 'Client log must be private and regular')
        with subprocess.Popen(args, cwd=root, env=environment, pass_fds=(lock.fileno(),),
                              stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors='replace') as process:
            def cancel(signum, frame):
                process.terminate()
                raise SystemExit(128 + signum)
            for signum in (signal.SIGTERM, signal.SIGINT):
                previous[signum] = signal.signal(signum, cancel)
            try:
                for line in process.stdout:
                    for secret in secrets:
                        line = line.replace(secret, '[REDACTED]')
                    log.write(line)
                    log.flush()
                result = process.wait()
            finally:
                if process.poll() is None:
                    process.terminate()
                    process.wait()
                for signum, handler in previous.items():
                    signal.signal(signum, handler)
        log.write('\nexitCode=' + str(result) + '\n')
        log.flush()
        os.fsync(log.fileno())
    require(result == 0, 'Client failed; preserve its original journal and deadline: ' + str(log_path))


def completed_transition(root, config, bundle, manifest):
    """Select only the transition named by its pinned, deployment-wide one-shot binding."""
    campaign = qualified_campaign(bundle, manifest['files']['verification.json'])
    if config.get('runtimeProfile') is not None:
        import runpy
        services = runpy.run_path(str(bundle / 'ops/setup-services.py'))
        services['runtime_profile'](config['runtimeProfile'], manifest['files']['bin/gear'])
        binding = json.loads(observer_binding_path(root, config).read_text())
        require(binding.get('schemaVersion') == 1 and binding.get('testOnly') is True
                and binding.get('runId') == root.name and binding.get('campaignName') == campaign
                and binding.get('bundleSha256') == config['bundle']['sha256']
                and binding.get('runtimeProfile') == config['runtimeProfile'] == manifest.get('runtimeProfile')
                and binding.get('automaticRerun') is False and binding.get('launched') is False,
                'Fresh normal campaign admission changed; retained lanes cannot attach')
        for relative, expected in binding['preservedFiles'].items():
            require(hashlib.sha256(contained(root, relative).read_bytes()).hexdigest() == expected,
                    'Normal campaign original identity changed: ' + relative)
        for relative, expected in binding['plists'].items():
            require(hashlib.sha256(contained(root, relative).read_bytes()).hexdigest() == expected,
                    'Normal campaign supervisor changed')
        for field, relative in (('runnerSha256', 'ops/run-preflight.py'), ('observerSha256', 'ops/warmup-supervisor-observer.py')):
            require(binding[field] == manifest['files'][relative], 'Normal runner/observer changed')
        launch = json.loads(contained(root, 'source-chain/launch-state.json').read_text())
        require(launch['identity'].get('runtimeProfile') == config['runtimeProfile'], 'Source profile differs from admission')
        return {'mutableFiles': {'qualification/components.json': {'new': binding['preservedFiles']['qualification/components.json']}}}, launch
    binding_relative = 'supervisors/hoodi-milestone-1-observer-binding.json'
    binding_path = contained(root, binding_relative)
    binding_bytes = binding_path.read_bytes()
    binding = json.loads(binding_bytes)
    name = binding.get('transitionName')
    import runpy
    services = runpy.run_path(str(bundle / 'ops/setup-services.py'))
    number = services['recovery_number'](name)
    record = contained(root, 'bundle-transitions/' + name + '/transition.json')
    require(binding.get('schemaVersion') == 1 and binding.get('testOnly') is True
            and binding.get('campaignName') == campaign and binding.get('bundleSha256') == config['bundle']['sha256']
            and binding.get('transition') == str(record) and binding.get('automaticRerun') is False
            and binding.get('launched') is False, 'Named campaign transition/bundle binding changed')
    for field, relative in (('runnerSha256', 'ops/run-preflight.py'), ('observerSha256', 'ops/warmup-supervisor-observer.py')):
        require(binding[field] == manifest['files'][relative]
                == hashlib.sha256((bundle / relative).read_bytes()).hexdigest(), 'Named observer/runner binding changed')
    transition = json.loads(record.read_text())
    require(transition.get('schemaVersion') == 1 and transition.get('testOnly') is True and transition.get('runId') == root.name
            and transition.get('transitionName') == name and transition.get('status') == 'PASS'
            and transition['newBundle'] == config['bundle']
            and all(transition[gate]['status'] == 'PASS' for gate in ('stopGate', 'sourceGate', 'progressGate')),
            'Fenced bound transition has not passed')
    require(transition['helperSha256'] == manifest['files']['ops/setup-services.py']
            == hashlib.sha256((bundle / 'ops/setup-services.py').read_bytes()).hexdigest(), 'Bound transition helper changed')
    require(hashlib.sha256(binding_bytes).hexdigest() == transition['mutableFiles'][binding_relative]['new'],
            'Named transition admission binding changed')
    plists = {'supervisors/' + campaign + '-' + mode + '.plist' for mode in ('preflight', 'warmup')}
    require(set(binding['plists']) == plists and binding['followerLabel'] == transition['actors']['follower']['label'],
            'Named one-shot/follower identity changed')
    for relative, expected in binding['plists'].items():
        require(expected == transition['mutableFiles'][relative]['new'], 'Named supervisor record changed')
    for relative, pins in transition['mutableFiles'].items():
        require(hashlib.sha256(contained(root, relative).read_bytes()).hexdigest() == pins['new'],
                'Selected component qualification changed' if relative == 'qualification/components.json'
                else 'Completed transition binding changed: ' + relative)
    if number:
        services['validate_recovery_record'](root, transition, record)
    launch_bytes = contained(root, 'bundle-transitions/' + name + '/source-after.json').read_bytes()
    require(hashlib.sha256(launch_bytes).hexdigest() == transition['sourceAfterSha256'], 'Transition source evidence changed')
    return transition, json.loads(launch_bytes)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('mode', choices=('preflight', 'warmup', 'app'))
    parser.add_argument('--campaign-name', required=True)
    parser.add_argument('--direction', choices=('eth-to-vara', 'vara-to-eth'))
    # Parse the delimiter separately: argparse REMAINDER would swallow wrapper admission flags.
    import sys
    argv = list(sys.argv[1:] if argv is None else argv)
    delimiter = argv.index('--') if '--' in argv else len(argv)
    options = parser.parse_args(argv[:delimiter])
    flags = argv[delimiter + 1:] if delimiter < len(argv) else []
    require(re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,63}', options.campaign_name) is not None, 'Invalid campaign name')
    app = app_arguments(flags) if options.mode == 'app' else None
    require((app is not None and options.direction is not None) or (not flags and options.direction is None),
            'Direction and example flags are app-only')
    from run_context import RUN as root, CONFIG as config, BUNDLE, MANIFEST, artifact, private_text
    require(options.campaign_name == qualified_campaign(BUNDLE, MANIFEST['files']['verification.json']),
            'Only the qualified Hoodi campaign is admitted')
    os.umask(0o077)
    with campaign_lock(root) as lock:
        campaign = campaign_directory(root, options.campaign_name, options.mode)
        transition, launch = completed_transition(root, config, BUNDLE, MANIFEST)
        validate_journal(root, campaign, config, launch)
        component_path = contained(root, 'qualification/components.json')
        require(hashlib.sha256(component_path.read_bytes()).hexdigest() == transition['mutableFiles']['qualification/components.json']['new'],
                'Selected component qualification changed')
        gate = json.loads(component_path.read_text())
        require(gate.get('status') == 'VERIFIED' and gate.get('bundleSha256') == config['bundle']['sha256'], 'Selected components are not qualified')
        environment = {key: os.environ[key] for key in ('PATH', 'HOME', 'BEEFY_RUN') if key in os.environ}
        environment['BEEFY_RUN'] = str(root)
        environment['BEEFY_CAMPAIGN_NAME'] = options.campaign_name
        environment['BEEFY_CAMPAIGN_LOCK_FD'] = str(lock.fileno())
        if app is not None:
            restart = json.loads((campaign / 'warmup-supervisor-restart.json').read_text())
            require(restart.get('phase') == 'native-warmup-and-real-restart-verified', 'Supervised restart qualification is missing')
            node, entry = app_artifact(root, options.campaign_name, options.direction, BUNDLE, MANIFEST['files']['verification.json'])
            if app['mode'] != 'verify':
                for key, relative in [('ETH_CAMPAIGN_KEY_FILE', 'hoodi/keys/campaign.key'),
                                      ('GEAR_CAMPAIGN_SURI_FILE', 'hoodi/gear-keys/campaign.suri')]:
                    path = contained(root, relative)
                    # Validate the private boundary here; TypeScript alone reads/signs with these files.
                    metadata = path.lstat()
                    require(stat.S_ISREG(metadata.st_mode) and stat.S_IMODE(metadata.st_mode) == 0o600
                            and 0 < metadata.st_size <= 16384, 'Campaign credential is not a private regular file')
                    environment[key] = str(path)
            run_child([str(node), str(entry), *flags], root, environment, lock,
                      root / 'hoodi' / ('app-' + options.direction + '.log'))
            return
        power = subprocess.check_output(['pmset', '-g', 'batt'], text=True, timeout=10)
        require("'AC Power'" in power, 'Timed bounded execution requires AC power; no campaign started')
        require(json.loads((root / 'hoodi/campaign-inventory-finalized.json').read_text())['phase'] == 'finalized', 'Inventory is not finalized')
        require(json.loads((root / 'source-chain/setup/bridge-ready.json').read_text())['phase'] == 'ready', 'Source bridge is not ready')
        require(json.loads((root / 'token-stack/token-stack.json').read_text())['configuration']['status'] == 'ready', 'Token configuration is not ready')
        binary = artifact('beefy-relay')
        readiness = root / 'source-chain' / ('readiness-' + options.campaign_name + '-' + options.mode + '-' + str(time.time_ns()) + '.json')
        subprocess.run([str(binary), 'tokens-source-state', '--raw-spec', str(root / 'source-chain/chain.raw.json'),
                        '--source-rpc', config['source']['aliceRpc'], '--witness-rpc', config['source']['bobRpc'],
                        '--output', str(readiness)], check=True, timeout=120)
        observed = json.loads(readiness.read_text())
        require(observed['phase'] == 'ready' and observed['identity'] == launch['identity'], 'Source identity changed')
        validate_journal(root, campaign, config, observed)
        secrets = []
        for role, key in [('campaign', 'GEAR_CAMPAIGN_SURI'), ('governance', 'GEAR_GOVERNANCE_SURI'), ('rotation', 'BEEFY_ROTATION_SURI')]:
            secret = private_text(contained(root, 'hoodi/gear-keys/' + role + '.suri'))
            environment[key] = secret
            secrets.append(secret)
        # Preserve the native output basename identity and its existing original deadlines.
        args = [str(binary), 'tokens-soak', '--mode', options.mode, '--rotation-authority', 'alice']
        network = config['network']
        values = {'source-rpc': observed['aliceRpc'], 'witness-rpc': observed['bobRpc'],
                  'ethereum-rpc': network['executionWss'], 'beacon-rpc': network['beaconHttp'],
                  'campaign-wallet': contained(root, 'hoodi/keys/campaign.key'), 'follower-dir': root / 'follower',
                  'inbound-dir': root / 'inbound', 'outbound-dir': root / 'outbound/journal',
                  'deployment-manifest': root / 'deployment.json', 'token-stack': root / 'token-stack/token-stack.json',
                  'raw-spec': root / 'source-chain/chain.raw.json', 'source-launch-state': readiness, 'output-dir': campaign}
        for key, value in values.items():
            args.extend(['--' + key, str(value)])
        run_child(args, root, environment, lock, root / 'hoodi' / (options.campaign_name + '-' + options.mode + '.log'), secrets)
        print('Bounded ' + options.mode + ' passed; no live application or mainnet qualification is implied.')


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Operational safety checks require non-optimized Python')
    main()
