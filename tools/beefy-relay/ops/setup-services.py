#!/usr/bin/env python3
"""Run this test lane's hash-pinned launchd jobs; load signing secrets only at exec time."""
import argparse
import contextlib
import fcntl
import hashlib
import json
import os
import plistlib
import re
import runpy
import secrets
import shutil
import socket
import stat
import subprocess
import time
import sys
from pathlib import Path


CAMPAIGN = 'hoodi-milestone-1'
TRANSITION = CAMPAIGN + '-observer'
RETAINED_RUN = 'f991e59c-71cb-4b0a-8c6c-56ce2acb8ac1'
GEAR_SHA = 'd25342d65033fdd0d2d04cb4302091aacf7a02c483844a195c71dad2d656b05e'
ACTORS = ('follower', 'outbound', 'inbound', 'checkpoint', 'alice', 'bob')
CHANGED_ARTIFACTS = {'bin/beefy-relay', 'verification.json', 'ops/run-preflight.py',
                     'ops/setup-services.py', 'ops/test_ops.py', 'ops/warmup-supervisor-observer.py'}
RECOVERY_ARTIFACTS = CHANGED_ARTIFACTS | {'bin/relayer'}
CONTINUATION_PROGRESS_MS = 120 * 60000
MILESTONE_CHECK_COMMANDS = {
    'python3 tools/beefy-relay/ops/test_ops.py',
    'cargo nextest run -p beefy-relay',
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


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()

def runtime_profile(profile, gear_sha256=None):
    """Validate a sealed independent artifact selection, never infer approval from RPC."""
    require(isinstance(profile, dict) and profile.get('name') in ('normal-runtime-hoodi', 'fast-runtime-hoodi'),
            'Unsupported runtime test profile; HOLD')
    fast = profile['name'] == 'fast-runtime-hoodi'
    require(profile.get('runtimeCommit') == '19ab81dd208b3ce7b339f5d1dbd144511d3c7e2c'
            and profile.get('runtimePullRequest') == 5642
            and profile.get('slotDurationMs') == 3000 and profile.get('epochDurationBlocks') == (64 if fast else 2400)
            and profile.get('warmupDurationMs') == (1 if fast else 5) * 60 * 60 * 1000
            and profile.get('requiredAuthorityHandovers') == 2
            and profile.get('tokenBatchDurationMs') == 60 * 60 * 1000
            and profile.get('applicationAttemptDurationMs') == 44 * 60 * 1000
            and profile.get('testOnly') is True and profile.get('executionAuthorized') is False
            and profile.get('releaseQualified') is False, 'Invalid runtime test profile; HOLD')
    require(profile.get('functionalOnly', False) is fast
            and (fast or 'cadencePatchSha256' not in profile), 'Fast functional evidence cannot represent normal-cadence acceptance; HOLD')
    if fast:
        patch = profile.get('cadencePatchSha256')
        require(isinstance(patch, str) and re.fullmatch(r'(?:0x)?[0-9a-f]{64}', patch) is not None
                and int(patch, 16) != 0, 'Fast runtime lacks its pinned cadence-only source patch; HOLD')
    for field in ('gearBinarySha256', 'runtimeCodeSha256', 'runtimeCodeBlake2b256', 'runtimeCodeKeccak256', 'approvalSha256'):
        require(isinstance(profile.get(field), str)
                and re.fullmatch(r'(?:0x)?[0-9a-f]{64}', profile[field]) is not None
                and int(profile[field], 16) != 0, 'Missing labeled approved artifact hash: ' + field)
    if gear_sha256 is not None:
        require(profile['gearBinarySha256'].removeprefix('0x') == gear_sha256.removeprefix('0x'),
                'Runtime executable differs from independently approved artifact')
    require(profile.get('runtimeCiStatus') in ('unresolved', 'failed', 'passed'), 'Runtime CI status must remain explicit')
    return profile

def normal_activation_authorities(current, next_keys, current_proof, next_proof):
    """Bound and authenticate current/next fixed SCALE frames before governed activation."""
    from Crypto.Hash import keccak
    def raw(value, maximum):
        require(isinstance(value, str) and len(value) <= 2 + maximum * 2
                and re.fullmatch(r'0x(?:[0-9a-fA-F]{2})+', value), 'Malformed/bounded authority frame; HOLD')
        return bytes.fromhex(value[2:])
    def keys(frame):
        require(frame and frame[0] & 3 in (0, 1), 'Noncanonical authority count; HOLD')
        width = 1 if frame[0] & 3 == 0 else 2
        require(len(frame) >= width, 'Truncated authority count; HOLD')
        count = int.from_bytes(frame[:width], 'little') >> 2
        require(1 <= count <= 256 and (width == 1) == (count < 64), 'Unsupported/noncanonical authority count; HOLD')
        end = width + count * 33
        require(len(frame) >= end, 'Truncated authority keys; HOLD')
        values = [frame[offset:offset+33] for offset in range(width, end, 33)]
        require(len(set(values)) == count, 'Duplicate authority keys; HOLD')
        addresses = []
        field = 2**256 - 2**32 - 977
        for value in values:
            x = int.from_bytes(value[1:], 'big')
            square = (pow(x, 3, field) + 7) % field
            y = pow(square, (field + 1) // 4, field)
            require(value[0] in (2, 3) and x < field and y*y % field == square, 'Invalid compressed authority key; HOLD')
            if y & 1 != value[0] & 1:
                y = field - y
            require(y < field, 'Invalid authority parity; HOLD')
            addresses.append(keccak.new(digest_bits=256, data=value[1:]+y.to_bytes(32, 'big')).digest()[-20:])
        require(len(set(addresses)) == count, 'Duplicate derived authority addresses; HOLD')
        layer = [keccak.new(digest_bits=256, data=address).digest() for address in addresses]
        while len(layer) > 1:
            layer = [keccak.new(digest_bits=256, data=layer[index]+layer[index+1]).digest()
                     if index+1 < len(layer) else layer[index] for index in range(0, len(layer), 2)]
        return count, layer[0], frame[end:]
    current = raw(current, 1+2+256*33+8)
    require(current[0] == 1, 'Current BEEFY authorities unavailable; HOLD')
    count, root, suffix = keys(current[1:])
    require(len(suffix) == 8, 'Trailing/malformed current authority set; HOLD')
    current_id = int.from_bytes(suffix, 'little')
    next_count, next_root, suffix = keys(raw(next_keys, 2+256*33))
    require(not suffix, 'Trailing next authority list; HOLD')
    for proof, identifier, length, commitment in ((current_proof, current_id, count, root), (next_proof, current_id+1, next_count, next_root)):
        proof = raw(proof, 44)
        require(len(proof) == 44 and int.from_bytes(proof[:8], 'little') == identifier
                and int.from_bytes(proof[8:12], 'little') == length and proof[12:] == commitment,
                'Authenticated authority commitment/id/count mismatch; HOLD')
    return {'currentId': current_id, 'currentCount': count, 'nextId': current_id+1, 'nextCount': next_count}


def load(path):
    return json.loads(Path(path).read_text())


def secure_path(root, relative):
    relative = Path(relative)
    require(not relative.is_absolute() and '..' not in relative.parts, 'Unsafe relative path')
    path = Path(root)
    for part in relative.parts:
        path = path / part
        require(not path.is_symlink(), 'Symlink in transition path: ' + str(relative))
    require(path.resolve().is_relative_to(Path(root).resolve()), 'Transition path escapes its owner')
    return path


def fsync_parent(path):
    fd = os.open(Path(path).parent, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def atomic_bytes(path, data):
    temporary = path.with_name(path.name + '.transition-tmp')
    with temporary.open('xb') as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())
    os.replace(temporary, path)
    fsync_parent(path)


def json_bytes(value):
    return (json.dumps(value, sort_keys=True, indent=2) + '\n').encode()


def persist(path, value):
    atomic_bytes(path, json_bytes(value))



def transition_journal(record):
    require(not record.is_symlink(), 'Symlink in transition journal')
    value = load(record) if record.exists() else None
    temporary = record.with_name(record.name + '.transition-tmp')
    require(not temporary.is_symlink(), 'Symlink in interrupted transition save')
    if not temporary.exists():
        return value, None
    mode = temporary.lstat().st_mode
    require(stat.S_ISREG(mode) and stat.S_IMODE(mode) == 0o600, 'Transition save is not a private regular file')
    raw = temporary.read_bytes()
    pending = json.loads(raw)
    require(raw == json_bytes(pending), 'Incomplete/noncanonical transition save; retain it and HOLD')
    if value is None:
        require(pending['phase'] == 'prepared' and not pending['starts'] and not pending['stopIntents'],
                'Orphan transition save has dispatched actors; HOLD')
    else:
        changing = {'status', 'phase', 'lastWallMs', 'stopGate', 'sourceGate', 'progressGate',
                    'stopIntents', 'starts', 'appliedObservation', 'historyAudit'}
        require(set(value) <= set(pending), 'Interrupted transition removed original evidence')
        for field, previous in value.items():
            if field not in changing:
                require(pending[field] == previous, 'Interrupted transition changed frozen evidence: ' + field)
        require(pending['lastWallMs'] >= value['lastWallMs'], 'Interrupted transition clock regressed')
        require(value['status'] != 'PASS' or pending == value, 'Terminal transition changed')
        for name in ('stopGate', 'sourceGate', 'progressGate'):
            before, after = value[name], pending[name]
            for field in ('name', 'startedAtMs', 'deadlineAtMs', 'absoluteCapAtMs'):
                if field in before:
                    require(after[field] == before[field], 'Interrupted transition reset an original gate')
            require(before['status'] != 'FAILED' or after['status'] == 'FAILED', 'Interrupted transition erased gate failure')
        source = pending['sourceGate']
        if 'startedAtMs' in source:
            require(source['deadlineAtMs'] == min(source['startedAtMs'] + 120000, source['absoluteCapAtMs']),
                    'Interrupted source gate widened its deadline')
        for name, intent in value['stopIntents'].items():
            require(pending['stopIntents'].get(name) == intent, 'Interrupted transition lost a stop intent')
        for name, started in value['starts'].items():
            current = pending['starts'].get(name)
            require(current is not None, 'Interrupted transition lost original actor identity')
            if 'pid' in started:
                process = current.get('process', {})
                require(current.get('pid') == started['pid'] and process.get('started') == started['process']['started']
                        and (process.get('pgid') == started['process']['pgid']
                             or started['status'] == 'intent' and started['process']['pgid'] == 1
                             and process.get('pgid') == started['pid']), 'Interrupted transition changed original actor process identity')
    return pending, (digest(record) if record.exists() else None, hashlib.sha256(raw).hexdigest(), raw)

def tree_pins(directory):
    pins = {}
    require(stat.S_ISDIR(directory.lstat().st_mode), 'Missing retained private tree')
    for path in sorted(directory.rglob('*')):
        mode = path.lstat().st_mode
        require(stat.S_ISREG(mode) or stat.S_ISDIR(mode), 'Unexpected retained tree entry: ' + str(path))
        relative = str(path.relative_to(directory))
        pins[relative] = {'kind': 'file' if stat.S_ISREG(mode) else 'directory', 'mode': stat.S_IMODE(mode)}
        if stat.S_ISREG(mode):
            pins[relative]['sha256'] = digest(path)
    return pins


def tree_digest(directory):
    return hashlib.sha256(json_bytes(tree_pins(directory))).hexdigest()


def authenticate_bundle(path, expected):
    path = Path(path).absolute()
    require(path == path.resolve(strict=True), 'Bundle path must be canonical and symlink-free')
    mode = path.lstat().st_mode
    require(stat.S_ISDIR(mode) and not mode & 0o222, 'Bundle is not immutable')
    manifest_path = secure_path(path, 'bundle.json')
    require(digest(manifest_path) == expected, 'Bundle descriptor digest mismatch')
    manifest = load(manifest_path)
    require(manifest['schemaVersion'] == 1 and manifest['testOnly'] is True, 'Only sealed test bundles are admitted')
    require(manifest['binaries'] == {name: 'bin/' + name for name in ('gear', 'beefy-relay', 'relayer', 'checkpoints-tool')},
            'Unexpected sealed executable layout')
    actual = set()
    for entry in path.rglob('*'):
        mode = entry.lstat().st_mode
        require(stat.S_ISREG(mode) or stat.S_ISDIR(mode), 'Symlink/special entry in sealed bundle')
        require(not mode & 0o222, 'Bundle is not immutable')
        if stat.S_ISREG(mode) and entry != manifest_path:
            actual.add(str(entry.relative_to(path)))
    require(actual == set(manifest['files']), 'Unpinned or missing sealed artifact')
    for relative, expected_hash in manifest['files'].items():
        require(digest(secure_path(path, relative)) == expected_hash, 'Changed sealed artifact: ' + relative)
    return manifest


def bundle_diff(old, candidate, recovery=False):
    removed = set(old['files']) - set(candidate['files'])
    added = set(candidate['files']) - set(old['files'])
    changed = {name for name in old['files'].keys() & candidate['files'].keys()
               if old['files'][name] != candidate['files'][name]}
    allowed = RECOVERY_ARTIFACTS if recovery else CHANGED_ARTIFACTS
    require(not removed and added <= (set() if recovery else {'ops/warmup-supervisor-observer.py'})
            and added | changed <= allowed, 'Candidate changes more than the reviewed ' + ('recovery ops' if recovery else 'observer') + ' slice')
    require(old['solidity'] == candidate['solidity'] and old['binaries'] == candidate['binaries'], 'Solidity/ABI binding changed')
    require(old['files']['bin/gear'] == candidate['files']['bin/gear'] == GEAR_SHA, 'Qualified Gear bytes changed')
    for name in (('checkpoints-tool',) if recovery else ('relayer', 'checkpoints-tool')):
        require(old['files']['bin/' + name] == candidate['files']['bin/' + name], 'Unreviewed native actor changed')
    return {name: {'old': old['files'].get(name), 'new': candidate['files'][name]} for name in sorted(added | changed)}


def qualify_candidate(bundle, manifest):
    evidence = load(bundle / 'verification.json')
    require(evidence.get('schemaVersion') == 1 and evidence.get('status') == 'VERIFIED', 'Candidate verification is not complete')
    checks = evidence['checks']
    require({'cargo-tests', 'forge-tests', 'full-release-build', 'historical-recovery'} <= {entry['name'] for entry in checks},
            'Required sealer checks are missing')
    require(MILESTONE_CHECK_COMMANDS <= {entry['command'] for entry in checks}, 'Milestone-1 offline checks are missing')
    for entry in checks:
        require(entry['exitCode'] == 0 and entry['command'] and digest(Path(entry['logPath'])) == entry['logSha256'],
                'Qualification command/log failed or changed')
    require(evidence['sourceFiles'], 'Qualified build sources are missing')
    for path, expected in evidence['sourceFiles'].items():
        require(digest(Path(path)) == expected, 'Qualified source/component bytes changed')
    require(set(evidence['binaries']) == set(manifest['binaries']), 'Qualified binary selection is incomplete')
    for name, entry in evidence['binaries'].items():
        require(entry['sha256'] == manifest['files']['bin/' + name], 'Qualified binary selection mismatch')
    return evidence


def qualify_native_changes(old, candidate, evidence):
    for binary, field, suite in (('beefy-relay', 'followerCatchupFix', 'beefy-nextest'),
                                 ('relayer', 'relayerSchedulingFix', 'cargo-tests')):
        previous, current = (bundle['files']['bin/' + binary] for bundle in (old, candidate))
        if previous == current:
            continue
        checks = {entry['name'] for entry in evidence['checks']}
        review = evidence.get(field)
        require(isinstance(review, dict), 'Changed native actor lacks a qualified review')
        require(review['previousSha256'] == previous and review['candidateSha256'] == current,
                'Native review does not bind both binary hashes')
        require(review['sourceFiles'] and all(evidence['sourceFiles'].get(path) == expected
                for path, expected in review['sourceFiles'].items()), 'Reviewed native sources are not qualified')
        require({suite, 'full-release-build', 'historical-recovery'} <= set(review['checks'])
                and set(review['checks']) <= checks, 'Native change lacks measured qualification checks')


def replacement_plist(raw, old, candidate):
    before = plistlib.loads(raw)
    after = dict(before)
    arguments = before['ProgramArguments']
    prefix = str(old / 'ops') + '/'
    changed = [str(candidate / 'ops') + '/' + item[len(prefix):] if item.startswith(prefix) else item for item in arguments]
    require(sum(item.startswith(prefix) for item in arguments) == 1, 'Expected exactly one sealed ops prefix')
    require(before.get('AbandonProcessGroup', False) is False, 'Original process-group cancellation is not safe')
    after['ProgramArguments'] = changed
    return raw if old == candidate else plistlib.dumps(after, sort_keys=False)


def cas_replace(path, old_hash, new_hash, new_bytes):
    require(hashlib.sha256(new_bytes).hexdigest() == new_hash, 'Recorded replacement bytes changed')
    current = digest(path) if path.exists() else None
    require(current in (old_hash, new_hash), 'Mutable file has a third digest; HOLD: ' + str(path))
    temporary = path.with_name(path.name + '.transition-tmp')
    require(not temporary.is_symlink(), 'Symlink in transition replacement')
    if current == new_hash:
        if temporary.exists():
            require(digest(temporary) == new_hash, 'Unresolved transition replacement; HOLD')
            temporary.unlink()  # Only our redundant, fully committed replacement, never a native save.
            fsync_parent(path)
        return
    if temporary.exists():
        require(not temporary.is_symlink() and digest(temporary) == new_hash, 'Unresolved transition replacement; HOLD')
        require((digest(path) if path.exists() else None) == old_hash, 'Mutable file changed during resume')
        os.replace(temporary, path)
        fsync_parent(path)
    else:
        require((digest(path) if path.exists() else None) == old_hash, 'Mutable file changed before CAS')
        atomic_bytes(path, new_bytes)


def clock_ms():
    return int(time.time() * 1000)


def remaining(gate, cap=10):
    require(gate.get('status') != 'FAILED', 'Original gate has already failed; no retry/reset')
    seconds = (gate['deadlineAtMs'] - clock_ms()) / 1000
    require(seconds > 0, 'Original ' + gate['name'] + ' deadline expired; readiness remains blocked')
    return min(cap, seconds)


def launch_observation(domain, label, timeout=10):
    result = subprocess.run(['/bin/launchctl', 'print', domain + '/' + label], capture_output=True, text=True, timeout=timeout)
    if result.returncode:
        require('Could not find service' in result.stderr, 'Supervisor observation failed, not absent: ' + label)
        return None
    match = re.search(r'^\s*pid = ([0-9]+)$', result.stdout, re.M)
    return {'label': label, 'pid': int(match[1]) if match else None, 'launchdSha256': hashlib.sha256(result.stdout.encode()).hexdigest()}


def process_table(timeout=10):
    text = subprocess.check_output(['/bin/ps', '-ww', '-axo', 'pid=,ppid=,pgid=,lstart=,command='], text=True, timeout=timeout)
    table = {}
    for line in text.splitlines():
        fields = line.split(maxsplit=8)
        if len(fields) == 9:
            table[int(fields[0])] = {'pid': int(fields[0]), 'ppid': int(fields[1]), 'pgid': int(fields[2]),
                                     'started': ' '.join(fields[3:8]), 'command': fields[8]}
    return table


def scoped_processes(table, root, checkpoint):
    parents, cursor = {os.getpid()}, os.getpid()
    while cursor in table and table[cursor]['ppid'] not in parents:
        cursor = table[cursor]['ppid']
        parents.add(cursor)
    return [value for pid, value in table.items() if pid not in parents
            and Path(value['command'].split()[0]).name != 'caffeinate'
            and (str(root) + '/' in value['command'] or ('eth-gear-core' in value['command'] and checkpoint in value['command']))]


def stop_barrier(root, domain, state, record, gate):
    if gate['status'] == 'PASS':
        return
    try:
        for name in ACTORS:
            actor = state['actors'][name]
            if name not in state['stopIntents']:
                state['stopIntents'][name] = {'atMs': clock_ms(), 'original': actor}
                persist(record, state)
            observation = launch_observation(domain, actor['label'], remaining(gate))
            if observation is not None:
                require(observation['pid'] == actor['pid'], 'Actor changed before fencing; HOLD')
                process = process_table(remaining(gate)).get(observation['pid'])
                require(process and process['started'] == actor['process']['started'], 'Actor process changed before fencing; HOLD')
                subprocess.run(['/bin/launchctl', 'bootout', domain + '/' + actor['label']],
                               capture_output=True, check=True, timeout=remaining(gate))
        while True:
            jobs = {name: launch_observation(domain, state['actors'][name]['label'], remaining(gate)) for name in ACTORS}
            processes = scoped_processes(process_table(remaining(gate)), root, state['checkpoint'])
            original_alive = [old for old in state['originalProcesses'] if old['pid'] in {entry['pid'] for entry in processes}
                              and any(entry['pid'] == old['pid'] and entry['started'] == old['started'] for entry in processes)]
            if not any(jobs.values()) and not processes and not original_alive:
                remaining(gate)
                gate.update(status='PASS', completedAtMs=clock_ms(), jobsAbsent=True, wrapperAndNativeExited=True)
                persist(record, state)
                return
            time.sleep(min(0.25, remaining(gate)))
    except BaseException as error:
        gate.update(status='FAILED' if gate['status'] == 'FAILED' or clock_ms() >= gate['deadlineAtMs'] else 'HOLD', reason=str(error))
        state['status'] = 'HOLD'
        persist(record, state)
        raise


def private_snapshot(root, destination, state, record):
    if state.get('snapshotFiles') is not None:
        require(tree_digest(destination) == state['snapshotTreeSha256'], 'Frozen worker snapshot changed')
        return
    require(not destination.exists(), 'Unfinished snapshot exists; retain it and HOLD')
    destination.mkdir(mode=0o700)
    for relative in ('follower', 'inbound', 'outbound/journal', 'checkpoint'):
        source = secure_path(root, relative)
        tree_pins(source)  # Refuse symlinks before copying any private state.
        target = destination / relative
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        shutil.copytree(source, target)
    pins = tree_pins(destination)
    require(all(digest(root / relative) == entry['sha256'] for relative, entry in pins.items() if entry['kind'] == 'file'),
            'Worker snapshot changed under ownership locks')
    for entry in destination.rglob('*'):
        if entry.is_file():
            entry.chmod(0o400)
            with entry.open('rb') as stream:
                os.fsync(stream.fileno())
    for entry in sorted((p for p in destination.rglob('*') if p.is_dir()), key=lambda p: len(p.parts), reverse=True):
        entry.chmod(0o500)
        fsync_parent(entry / 'entry')
    destination.chmod(0o500)
    fsync_parent(destination)
    state['snapshotFiles'] = {name: entry['sha256'] for name, entry in pins.items() if entry['kind'] == 'file'}
    state['snapshotTreeSha256'] = tree_digest(destination)
    state['unfinishedNativeSaves'] = [name for name in state['snapshotFiles'] if name.endswith('.tmp')]
    persist(record, state)


def native_source_state(binary, root, config, output, timeout):
    subprocess.run([str(binary), 'tokens-source-state', '--raw-spec', str(root / 'source-chain/chain.raw.json'),
                    '--source-rpc', config['source']['aliceRpc'], '--witness-rpc', config['source']['bobRpc'],
                    '--output', str(output)], check=True, timeout=timeout)
    value = load(output)
    require(value['phase'] == 'ready', 'Native source state is not ready')
    return value


def authenticate_history(config, anchor, before, timeout=10):
    from substrateinterface import SubstrateInterface
    result = {}
    for endpoint in (config['source']['aliceRpc'], config['source']['bobRpc']):
        api = SubstrateInterface(url=endpoint, ws_options={'timeout': timeout})
        try:
            require(api.get_block_hash(0) == anchor['sourceGenesis'] and api.get_block_hash(anchor['block']) == anchor['blockHash'],
                    'Source genesis/old anchor changed')
            head = api.get_chain_finalised_head()
            height = api.get_block_number(head)
            if before:
                require(height >= before[endpoint]['height'] and api.get_block_hash(before[endpoint]['height']) == before[endpoint]['hash'],
                        'Pre-stop finalized history regressed/changed')
            result[endpoint] = {'height': height, 'hash': head}
        finally:
            api.close()
    common = min(value['height'] for value in result.values())
    for endpoint, value in result.items():
        api = SubstrateInterface(url=endpoint, ws_options={'timeout': timeout})
        try:
            value['commonHeight'], value['commonHash'] = common, api.get_block_hash(common)
        finally:
            api.close()
    require(len({value['commonHash'] for value in result.values()}) == 1, 'Source/witness finalized history diverged')
    return result


def embedded_programs(old, candidate, evidence, stack, checkpoint, config, source_pin):
    import mmap
    from substrateinterface import SubstrateInterface
    proof = evidence['embeddedPrograms']
    require(proof['status'] == 'VERIFIED', 'Embedded program qualification is missing')
    programs = {entry['name']: entry for entry in proof['programs']}
    bindings = {'historical_proxy': [stack['programs']['historicalProxy']['id']],
                'eth_events_electra': [stack['programs']['ethEventsElectra']['id']],
                'vft_manager': [stack['programs']['vftManager']['id']],
                'bridging_payment': [stack['programs']['bridgingPayment']['id']],
                'vft': [stack['programs'][name]['id'] for name in ('circleVft', 'tetherVft', 'etherVft', 'bitcoinVft', 'gearOriginVft')],
                'vft_vara': [stack['programs']['nativeVft']['id']], 'checkpoint_light_client': [checkpoint]}
    require(set(programs) == set(bindings), 'Incomplete deployed program byte qualification')
    observed = {}
    for name, ids in bindings.items():
        entry = programs[name]
        blob = Path(entry['path']).read_bytes()
        code_id = '0x' + hashlib.blake2b(blob, digest_size=32).hexdigest()
        require(code_id.removeprefix('0x') == entry['codeId'].removeprefix('0x')
                and hashlib.sha256(blob).hexdigest() == entry['sha256'], 'Generated deployed WASM blob changed')
        binary = 'checkpoints-tool' if name == 'checkpoint_light_client' else 'beefy-relay'
        for bundle in (old, candidate):
            with (bundle / 'bin' / binary).open('rb') as stream, mmap.mmap(stream.fileno(), 0, access=mmap.ACCESS_READ) as contents:
                require(contents.find(blob) >= 0, 'Exact deployed program bytes are absent from selected executable')
        for endpoint in (config['source']['aliceRpc'], config['source']['bobRpc']):
            api = SubstrateInterface(url=endpoint, ws_options={'timeout': 10})
            try:
                for program in ids:
                    value = api.query('GearProgram', 'ProgramStorage', [program], block_hash=source_pin).value
                    require(isinstance(value, dict) and value.get('Active', {}).get('code_id') == code_id,
                            'Deployed CodeId does not equal qualified embedded WASM')
            finally:
                api.close()
        observed[name] = {'codeId': code_id, 'sha256': entry['sha256'], 'programIds': ids}
    return observed


def source_gate(root, candidate, config, state, record):
    gate = state['sourceGate']
    if gate['status'] == 'PASS':
        return
    try:
        output = record.parent / 'source-after.json'
        probe = output if not output.exists() else record.parent / ('source-observation-' + str(time.time_ns()) + '.json')
        while True:
            remaining(gate)
            try:
                observed = native_source_state(candidate / 'bin/beefy-relay', root, config, probe, remaining(gate, 120))
                break
            except (subprocess.CalledProcessError, ConnectionError, OSError):
                require(not probe.exists(), 'Incomplete native source observation; retain it and HOLD')
                time.sleep(min(2, remaining(gate)))
        expected_identity = {**load(record.parent / 'source-before.json')['identity'],
                             'relayBinarySha256': '0x' + digest(candidate / 'bin/beefy-relay')}
        require(observed['identity'] == expected_identity, 'Source identity changed beyond observer relay bytes')
        heads = authenticate_history(config, load(root / 'anchor.json'), state['sourceBefore'], remaining(gate))
        retained = load(output)
        require(retained['phase'] == 'ready' and retained['identity'] == expected_identity, 'Retained source-after evidence changed')
        common = retained['readiness']['commonFinalized']
        authenticate_history(config, load(root / 'anchor.json'),
            {endpoint: {'height': common['height'], 'hash': common['hash']}
             for endpoint in (config['source']['aliceRpc'], config['source']['bobRpc'])}, remaining(gate))
        sets = observed['readiness']['authoritySets']
        require(sets['current']['length'] == sets['next']['length'] == 2
                and sets['next']['id'] == sets['current']['id'] + 1, 'Current/next source sets are invalid')
        remaining(gate)
        gate.update(status='PASS', completedAtMs=clock_ms())
        state['sourceAfter'] = heads
        state['sourceAfterSha256'] = digest(output)
        persist(record, state)
    except BaseException as error:
        gate.update(status='FAILED' if gate['status'] == 'FAILED' or clock_ms() >= gate['deadlineAtMs'] else 'HOLD', reason=str(error))
        persist(record, state)
        raise


def start_actor(root, candidate, domain, name, state, record, gate):
    actor = state['actors'][name]
    path = root / 'supervisors' / (name + '.plist')
    observation = launch_observation(domain, actor['label'], remaining(gate))
    previous = state['starts'].get(name)
    require(observation is None or previous is not None, 'Unrecorded actor is already loaded; HOLD')
    if previous and 'pid' in previous:
        require(observation and observation['pid'] == previous['pid'], 'Already-started actor identity changed; HOLD')
    if observation is None:
        require(previous is None, 'Previously dispatched actor is absent; do not restart it again')
        state['starts'][name] = {'status': 'intent', 'atMs': clock_ms()}
        persist(record, state)
        subprocess.run(['/bin/launchctl', 'bootstrap', domain, str(path)], capture_output=True, check=True, timeout=remaining(gate))
        observation = launch_observation(domain, actor['label'], remaining(gate))
    binary = 'gear' if name in ('alice', 'bob') else 'beefy-relay' if name == 'follower' else 'relayer'
    native = str(candidate / 'bin' / binary)
    while True:
        remaining(gate)
        require(observation is not None, 'Previously dispatched actor disappeared; do not restart it again')
        previous = state['starts'][name]
        if 'pid' in previous:
            require(observation['pid'] == previous['pid'], 'Already-started actor identity changed; HOLD')
        if observation['pid']:
            table = process_table(remaining(gate))
            require(observation['pid'] in table, 'Supervised actor process is absent')
            process = table[observation['pid']]
            if 'pid' in previous:
                require(process['started'] == previous['process']['started']
                        and (process['pgid'] == previous['process']['pgid']
                             or previous['status'] == 'intent' and previous['process']['pgid'] == 1
                             and process['pgid'] == previous['pid']), 'Already-started actor process identity changed; HOLD')
                if process['pgid'] != previous['process']['pgid']:
                    previous['process'] = process
                    persist(record, state)
            else:
                previous.update(**observation, process=process)
                persist(record, state)
            command = process['command']
            if process['pgid'] == process['pid'] and (command == native or command.startswith(native + ' ')):
                remaining(gate)
                state['starts'][name] = {**previous, 'status': 'authenticated', **observation, 'process': process}
                state['starts'][name].setdefault('authenticatedAtMs', clock_ms())
                persist(record, state)
                return
            require(previous['status'] != 'authenticated', 'Actor did not apply its exact candidate sealed wrapper/executable')
        # launchd may expose Python's platform executable before the sealed wrapper execs the actor.
        time.sleep(min(0.1, remaining(gate)))
        observation = launch_observation(domain, actor['label'], remaining(gate))


def request_json(url, body=None, timeout=10):
    import urllib.request
    request = urllib.request.Request(url, None if body is None else json.dumps(body).encode(),
                                     {'Content-Type': 'application/json', 'User-Agent': 'Mozilla/5.0 (beefy-hoodi-ops)'})
    with urllib.request.urlopen(request, timeout=timeout) as response:
        return json.load(response)


def execution_rpc(config, method, params, timeout=10):
    reply = request_json(config['network']['executionHttp'], {'jsonrpc': '2.0', 'id': 1, 'method': method, 'params': params}, timeout)
    require(reply.get('id') == 1 and 'result' in reply and 'error' not in reply, 'Execution RPC failed: ' + method)
    return reply['result']


def execution_state(config, deployment, timeout=10):
    from eth_utils import keccak
    def rpc(method, params):
        return execution_rpc(config, method, params, timeout)
    require(int(rpc('eth_chainId', []), 16) == 560048
            and rpc('eth_getBlockByNumber', ['0x0', False])['hash'] == config['network']['genesisHash'], 'Not public Hoodi')
    block = rpc('eth_getBlockByNumber', ['finalized', False])
    pin = {'blockHash': block['hash'], 'requireCanonical': True}
    for field in ('client', 'verifier', 'queue', 'receiver'):
        code = bytes.fromhex(rpc('eth_getCode', [deployment[field], pin])[2:])
        require(code and '0x' + keccak(code).hex() == deployment['bytecodeHashes'][field], 'Deployed execution code changed')
    def call(address, signature):
        result = rpc('eth_call', [{'to': address, 'data': '0x' + keccak(text=signature)[:4].hex()}, pin])
        require(re.fullmatch('0x[0-9a-fA-F]{64}', result) is not None, 'Unexpected execution getter ABI')
        return int(result, 16)
    return {'finalizedHash': block['hash'], 'finalizedHeight': int(block['number'], 16),
            'beefyBlock': call(deployment['client'], 'latestBeefyBlock()'),
            'queueBlock': call(deployment['queue'], 'maxBlockNumber()')}


def startup_progress_ready(state, execution, checkpoint, follower):
    progressed = (execution['beefyBlock'] > state['executionBefore']['beefyBlock']
                  and checkpoint['slot'] > state['checkpointBefore']['slot'] and not checkpoint['replaying']
                  and follower['follower']['status'] == 'healthy')
    if recovery_number(state['transitionName']):
        target = state['rootScanTarget']['height']
        return (progressed and execution['queueBlock'] >= state['executionBefore']['queueBlock']
                and follower['rootScan']['block'] >= target and execution['beefyBlock'] >= target
                and follower['follower']['lastFinalizedUpdate'] >= target)
    return progressed and execution['queueBlock'] > state['executionBefore']['queueBlock']


def applied_checkpoint(config, checkpoint, heads, timeout=10):
    import struct
    from substrateinterface import SubstrateInterface
    route = bytes([48]) + b'ServiceState' + bytes([12]) + b'Get'
    payload = '0x' + (route + b'\x01' + struct.pack('<II', 0, 1)).hex()
    states = []
    for endpoint in (config['source']['aliceRpc'], config['source']['bobRpc']):
        api = SubstrateInterface(url=endpoint, ws_options={'timeout': timeout})
        try:
            reply = api.rpc_request('gear_calculateReplyForHandle', ['0x' + '00' * 32, checkpoint, payload,
                                                                   750000000000, 0, heads[endpoint]['commonHash']])
            require('error' not in reply and reply['result']['code'] == {'Success': 'Manual'}, 'Applied checkpoint query failed')
            raw = bytes.fromhex(reply['result']['payload'][2:])
            require(raw[:17] == route and len(raw) in (59, 75) and raw[17] == 4 and raw[58] in (0, 1)
                    and len(raw) == (59 if raw[58] == 0 else 75), 'Unknown deployed checkpoint state ABI')
            states.append({'slot': struct.unpack_from('<Q', raw, 18)[0], 'root': '0x' + raw[26:58].hex(), 'replaying': bool(raw[58])})
        finally:
            api.close()
    require(states[0] == states[1], 'Applied checkpoint source/witness disagreement')
    header = request_json(config['network']['beaconHttp'] + '/eth/v1/beacon/headers/' + str(states[0]['slot']), timeout=timeout)
    require(header.get('execution_optimistic') is False and header.get('finalized') is True
            and header['data']['root'] == states[0]['root'], 'Applied checkpoint root is not canonical finalized Beacon state')
    return states[0]


def read_snapshot(binary, root, config, output, timeout=120):
    gear = load(root / 'hoodi/gear-addresses.json')['roles']['campaign']['publicKey']
    evm = load(root / 'hoodi/addresses.json')['roles']['campaign']
    args = [str(binary), 'tokens-snapshot', '--source-rpc', config['source']['aliceRpc'], '--witness-rpc', config['source']['bobRpc'],
            '--ethereum-rpc', config['network']['executionWss'], '--beacon-rpc', config['network']['beaconHttp'],
            '--deployment-manifest', str(root / 'deployment.json'), '--token-stack', str(root / 'token-stack/token-stack.json'),
            '--gear-user', gear, '--evm-user', evm, '--output', str(output)]
    subprocess.run(args, check=True, timeout=timeout)
    snapshot = load(output)
    assets = snapshot['assets']
    require(set(assets) == {'USDC', 'USDT', 'WBTC', 'WETH'}, 'Incomplete original asset snapshot')
    require(all(item['gearUser'] == item['vftSupply'] == item['evmManagerEscrow'] == '0' and int(item['evmUser']) >= 24
                for item in assets.values()), 'Original economic outcomes/inventory are unresolved; no submission')
    return snapshot


def one_shots(root, candidate, state, record, campaign):
    paths = {}
    labels = {}
    for mode in ('preflight', 'warmup'):
        original = load_plist(root / 'supervisors' / (mode + '-one-shot.plist'))
        require(original.get('KeepAlive') is False and original.get('RunAtLoad') is True
                and original.get('AbandonProcessGroup', False) is False, 'Unsafe original one-shot cancellation policy')
        new = dict(original)
        label = original['Label'] + '.' + campaign
        new['Label'] = label
        new['ProgramArguments'] = [original['ProgramArguments'][0], str(candidate / 'ops' / (
            'run-preflight.py' if mode == 'preflight' else 'warmup-supervisor-observer.py'))]
        if mode == 'preflight':
            new['ProgramArguments'].append('preflight')
        new['ProgramArguments'] += ['--campaign-name', campaign]
        for key in ('StandardOutPath', 'StandardErrorPath'):
            require(Path(original[key]).is_relative_to(root), 'Original launch log escapes the run')
            new[key] = str(root / 'hoodi' / (campaign + '-' + mode + '-supervisor.' + ('stdout' if key == 'StandardOutPath' else 'stderr') + '.log'))
        relative = 'supervisors/' + campaign + '-' + mode + '.plist'
        data = plistlib.dumps(new, sort_keys=False)
        paths[relative] = data
        labels[mode] = label
    binding = {'schemaVersion': 1, 'testOnly': True, 'campaignName': campaign, 'bundleSha256': state['newBundle']['sha256'],
               'runnerSha256': digest(candidate / 'ops/run-preflight.py'), 'observerSha256': digest(candidate / 'ops/warmup-supervisor-observer.py'),
               'followerLabel': state['actors']['follower']['label'], 'preflightLabel': labels['preflight'], 'warmupLabel': labels['warmup'],
               'plists': {relative: hashlib.sha256(data).hexdigest() for relative, data in paths.items()},
               'transition': str(record), 'transitionName': state['transitionName'], 'automaticRerun': False, 'launched': False}
    paths['supervisors/hoodi-milestone-1-observer-binding.json'] = json_bytes(binding)
    return paths


def load_plist(path):
    return plistlib.loads(path.read_bytes())


def recovery_number(name):
    require(isinstance(name, str) and len(name) <= 64, 'Invalid transition name')
    if name == TRANSITION:
        return 0
    match = re.fullmatch(CAMPAIGN + r'-recovery-([1-9][0-9]*)', name)
    require(match is not None, 'Invalid Hoodi continuation name')
    return int(match[1])


def pinned_evidence(binding):
    path = Path(binding['path'])
    require(path.is_absolute() and path == path.resolve(strict=True) and stat.S_ISREG(path.lstat().st_mode)
            and re.fullmatch('[0-9a-f]{64}', binding['sha256']) is not None
            and digest(path) == binding['sha256'], 'Pinned continuation evidence changed')
    return load(path)


def qualify_application_rebind(root, old, candidate, old_manifest, new_manifest, admission):
    manifests = []
    for bundle, manifest in ((old, old_manifest), (candidate, new_manifest)):
        campaign = admission['qualified_campaign'](bundle, manifest['files']['verification.json'])
        node, _ = admission['app_artifact'](root, campaign, 'eth-to-vara', bundle, manifest['files']['verification.json'])
        value = load(node.parent.parent / 'manifest.json')
        manifests.append({key: item for key, item in value.items() if key != 'campaignName'})
    require(manifests[0] == manifests[1], 'Continuation application artifacts changed beyond campaignName')


def recovery_one_shots(bundle, state, record):
    directory = Path(bundle['path'])
    manifest = authenticate_bundle(directory, bundle['sha256'])
    admission = runpy.run_path(str(Path(__file__).with_name('run-preflight.py')))
    campaign = admission['qualified_campaign'](directory, manifest['files']['verification.json'])
    relative = 'supervisors/hoodi-milestone-1-observer-binding.json'
    path = secure_path(record.parent, 'after/' + relative)
    require(digest(path) == state['mutableFiles'][relative]['new'], 'Pinned one-shot binding snapshot changed')
    binding = load(path)
    require(binding['schemaVersion'] == 1 and binding['testOnly'] is True and binding['campaignName'] == campaign
            and binding['bundleSha256'] == bundle['sha256'] and binding['transition'] == str(record)
            and binding['transitionName'] == state['transitionName'] and binding['automaticRerun'] is False
            and binding['launched'] is False and binding['followerLabel'] == state['actors']['follower']['label'],
            'Pinned current one-shot binding does not identify its predecessor')
    for field, name in (('runnerSha256', 'ops/run-preflight.py'), ('observerSha256', 'ops/warmup-supervisor-observer.py')):
        require(binding[field] == manifest['files'][name], 'Pinned predecessor runner/observer changed')
    require(state['helperSha256'] == manifest['files']['ops/setup-services.py'], 'Predecessor helper changed')
    require(set(binding['plists']) == {'supervisors/' + campaign + '-' + mode + '.plist' for mode in ('preflight', 'warmup')}
            and all(expected == state['mutableFiles'][name]['new'] for name, expected in binding['plists'].items()),
            'Pinned predecessor one-shot inventory changed')
    return binding



def continuation_authorization(root, binding):
    value = pinned_evidence(binding)
    require(value['schemaVersion'] == 1 and value['testOnly'] is True and value['runId'] == root.name
            and value['routineHoodiFixesRestartsDeploymentsAndSeparateTestAttemptsAuthorized'] is True
            and value['refillAndDistributionAuthorized'] is True and value['failedHistoryPreserved'] is True
            and all(value[field] is False for field in ('mainnetSigningAuthorized', 'mainnetActivationAuthorized',
                    'resetFailedDeadlines', 'replaceUnresolvedSignedIntents'))
            and value['distributionAddress'].lower() == load(root / 'run.json')['funding']['address'].lower(),
            'Continuation authorization is not custody-preserving Hoodi-only scope')
    return binding


def frozen_recovery_record(state, directory):
    path = secure_path(directory, 'recovery-authorization.json')
    require(stat.S_ISREG(path.lstat().st_mode) and stat.S_IMODE(path.stat().st_mode) == 0o400
            and digest(path) == state['recoveryAuthorizationSha256'], 'Frozen continuation bindings changed')
    frozen = load(path)
    for field, value in frozen.items():
        actual = state[field]
        if field in ('stopGate', 'sourceGate', 'progressGate'):
            actual = {key: actual[key] for key in value}
        require(actual == value, 'Recovery frozen candidate/helper/actor/deadline bindings changed: ' + field)


def continuation_predecessor_pin(root, verification, state=None, expected_sha=None):
    if state is not None:
        pin = state['predecessor']
        require(expected_sha is None or expected_sha == pin['sha256'], 'Resume predecessor digest changed')
        return pin
    if expected_sha is None:
        return verification['continuationPredecessor']
    require(re.fullmatch('[0-9a-f]{64}', expected_sha) is not None, 'Predecessor SHA256 must be exact lowercase hex')
    binding = load(secure_path(root, 'supervisors/hoodi-milestone-1-observer-binding.json'))
    return {'path': binding['transition'], 'sha256': expected_sha}


def recovery_successor(root, bundle, previous, predecessor, successor_name, state=None):
    """Skip only authenticated, expired preparations that never installed their descriptors."""
    installed, number = recovery_number(previous['transitionName']), recovery_number(successor_name)
    require(number > installed, 'Continuation must follow its installed predecessor')
    history = secure_path(root, 'bundle-transitions')
    paths, legacy = {}, {}
    for entry in history.iterdir():
        path = secure_path(root, 'bundle-transitions/' + entry.name)
        require(stat.S_ISDIR(path.lstat().st_mode), 'Ambiguous continuation history inventory')
        if path.name == TRANSITION or path.name.startswith(CAMPAIGN + '-recovery-'):
            paths[recovery_number(path.name)] = path
        else:
            legacy[str(path.relative_to(root))] = tree_digest(path)
    count = number + (state is not None)
    require(len(paths) == count and set(paths) == set(range(count)),
            'Continuation history inventory is missing, active or ambiguous; HOLD')
    trees = {index: (str(path.relative_to(root)), tree_digest(path)) for index, path in paths.items() if index < number}
    earlier = lambda limit: {**legacy, **{relative: sha for index, (relative, sha) in trees.items() if index < limit}}
    if installed:
        require(previous['historyTrees'] == earlier(installed), 'Installed predecessor history changed')
    if state is not None:
        require(state['historyTrees'] == earlier(number), 'Continuation history changed before resume')
    old_manifest = authenticate_bundle(Path(bundle['path']), bundle['sha256']) if number > installed + 1 else None
    for index in range(installed + 1, number):
        directory = paths[index]
        record = secure_path(directory, 'transition.json')
        require(record.is_file(), 'Missing continuation history journal; HOLD')
        require(not record.with_name(record.name + '.transition-tmp').exists(), 'Unresolved continuation history save; HOLD')
        failed = load(record)
        require(failed['schemaVersion'] == 1 and failed['testOnly'] is True and failed['runId'] == root.name
                and failed['transitionName'] == directory.name and failed['oldBundle'] == bundle
                and failed['predecessor'] == predecessor, 'Unbound continuation identity/baseline changed')
        frozen_recovery_record(failed, directory)
        require(load(directory / 'recovery-authorization.json') == recovery_identity(failed), 'Recovery frozen identity is incomplete')
        require(failed['status'] == 'HOLD' and failed['phase'] == 'prepared' and not failed['starts']
                and failed['stopGate']['status'] == 'FAILED'
                and failed['sourceGate']['status'] == failed['progressGate']['status'] == 'PENDING'
                and all(failed[field] is False for field in ('campaignsLaunched', 'nativeJournalManuallyEdited',
                        'fundingPerformedByTransition', 'identitiesReset')),
                'Intervening continuation is active, bound or ambiguous; resume its existing owner')
        prepared = failed['preparedAtMs']
        require(failed['stopGate']['startedAtMs'] == prepared and failed['stopGate']['deadlineAtMs'] == prepared + 30000
                and failed['sourceGate']['absoluteCapAtMs'] == prepared + 150000
                and failed['progressGate']['deadlineAtMs'] == prepared + CONTINUATION_PROGRESS_MS
                and clock_ms() >= max(failed['stopGate']['deadlineAtMs'], failed['lastWallMs']),
                'Intervening continuation has not expired or its original deadlines changed')
        require(not any(field in failed[gate] for gate in ('stopGate', 'sourceGate', 'progressGate')
                        for field in ('completedAtMs', 'jobsAbsent', 'wrapperAndNativeExited'))
                and 'startedAtMs' not in failed['sourceGate'] and 'deadlineAtMs' not in failed['sourceGate']
                and not any(field in failed for field in ('snapshotFiles', 'snapshotTreeSha256', 'unfinishedNativeSaves',
                        'sourceAfter', 'sourceAfterSha256', 'appliedObservation', 'historyAudit', 'queueRootContinuity'))
                and not (directory / 'before-runtime').exists() and not (directory / 'source-after.json').exists(),
                'Intervening continuation completed later work; HOLD')
        candidate = Path(failed['newBundle']['path'])
        manifest = authenticate_bundle(candidate, failed['newBundle']['sha256'])
        require(failed['helperSha256'] == manifest['files']['ops/setup-services.py']
                and failed['allowlistedArtifactDiff'] == bundle_diff(old_manifest, manifest, True),
                'Unbound continuation bundle/helper changed')
        continuation_authorization(root, failed['continuationAuthorization'])
        require(failed['historyTrees'] == earlier(index), 'Intervening immutable history changed')
        require(digest(secure_path(directory, 'source-before.json')) == failed['sourceBeforeSha256']
                and tree_digest(root / 'campaign') == failed['originalCampaignTreeSha256'], 'Intervening source/campaign history changed')
        for relative, expected in failed['preservedFiles'].items():
            require(digest(secure_path(root, relative)) == expected, 'Intervening preserved history changed: ' + relative)
        for relative, expected in failed['preservedArtifactTrees'].items():
            require(tree_digest(secure_path(root, relative)) == expected, 'Intervening application history changed')
        binding = recovery_one_shots(failed['newBundle'], failed, record)
        extra = set(failed['mutableFiles']) - set(previous['mutableFiles'])
        require(set(previous['mutableFiles']) <= set(failed['mutableFiles'])
                and (not extra or extra == set(binding['plists'])), 'Intervening descriptor inventory changed')
        for relative, pins in failed['mutableFiles'].items():
            before = secure_path(directory, 'before/' + relative)
            expected = previous['mutableFiles'][relative]['new'] if relative not in extra else None
            require(pins['old'] == expected
                    and (digest(before) == expected if expected is not None else not before.exists())
                    and digest(secure_path(directory, 'after/' + relative)) == pins['new'],
                    'Intervening descriptor replacement/baseline evidence changed')
            path = secure_path(root, relative)
            require(not path.with_name(path.name + '.transition-tmp').exists(), 'Unresolved intervening descriptor save; HOLD')
            if state is None:
                require((digest(path) if path.exists() else None) == pins['old'],
                        'Intervening descriptor changes are partially applied or unresolved; HOLD')
            else:
                require((relative in state['mutableFiles'] and state['mutableFiles'][relative]['old'] == pins['old']
                         or relative in extra and not path.exists()), 'Continuation lost unapplied predecessor baseline')


def recovery_predecessor(root, bundle, expected, state=None, successor_name=None):
    """Authenticate the selected journal, not a historically fixed baseline or stale PIDs."""
    binding_relative = 'supervisors/hoodi-milestone-1-observer-binding.json'
    path = Path(expected['path'])
    require(path.is_absolute() and path == secure_path(root, 'bundle-transitions/' + path.parent.name + '/transition.json'),
            'Predecessor journal escapes the retained transition owner')
    previous = pinned_evidence(expected)
    number = recovery_number(previous['transitionName'])
    require(path.parent.name == previous['transitionName'] and not path.with_name(path.name + '.transition-tmp').exists(),
            'Predecessor save/identity is unresolved; HOLD')
    require(previous['schemaVersion'] == 1 and previous['testOnly'] is True and previous['runId'] == root.name
            and previous['newBundle'] == bundle and previous['phase'] in
            ('descriptor-and-supervisors-bound', 'applied-progress-and-originals-verified')
            and (previous['status'] == 'PASS' and all(previous[gate]['status'] == 'PASS'
                    for gate in ('stopGate', 'sourceGate', 'progressGate'))
                 or previous['status'] == 'HOLD' and (any(previous[gate]['status'] == 'FAILED'
                    for gate in ('stopGate', 'sourceGate', 'progressGate'))
                    or previous['stopGate']['status'] == previous['sourceGate']['status'] == 'PASS'
                    and previous['progressGate']['status'] == 'HOLD'
                    and isinstance(previous['progressGate'].get('reason'), str) and bool(previous['progressGate']['reason']))),
            'Predecessor is neither completed nor an immutable failed attempt; resume its existing owner')
    require(previous['stopGate']['status'] == 'PASS' and previous['stopGate']['jobsAbsent'] is True
            and previous['stopGate']['wrapperAndNativeExited'] is True and not previous['unfinishedNativeSaves']
            and previous['campaignsLaunched'] is False and previous['nativeJournalManuallyEdited'] is False
            and previous['identitiesReset'] is False, 'Predecessor lacks authentic fencing/preserved state')
    if number:
        frozen_recovery_record(previous, path.parent)
        ancestor = previous['predecessor']
        require(digest(Path(ancestor['path'])) == ancestor['sha256']
                and tree_digest(Path(ancestor['path']).parent) == ancestor['treeSha256'], 'Earlier failed history changed')
    require(digest(path.parent / 'source-before.json') == previous['sourceBeforeSha256']
            and tree_digest(path.parent / 'before-runtime') == previous['snapshotTreeSha256']
            and tree_digest(root / 'campaign') == previous['originalCampaignTreeSha256'], 'Predecessor private/history evidence changed')
    if 'sourceAfterSha256' in previous:
        require(digest(path.parent / 'source-after.json') == previous['sourceAfterSha256'], 'Predecessor applied source evidence changed')
    for relative, expected_hash in previous['preservedFiles'].items():
        require(digest(secure_path(root, relative)) == expected_hash, 'Protected predecessor binding changed: ' + relative)
    for relative, expected_hash in previous.get('preservedArtifactTrees', {}).items():
        require(tree_digest(secure_path(root, relative)) == expected_hash, 'Preserved application artifact tree changed')
    for relative, pins in previous['mutableFiles'].items():
        require(digest(secure_path(path.parent, 'after/' + relative)) == pins['new'], 'Predecessor replacement evidence changed')
        if state is None:
            require(digest(secure_path(root, relative)) == pins['new'], 'Predecessor applied binding changed: ' + relative)
        else:
            require(state['mutableFiles'][relative]['old'] == pins['new'], 'Continuation lost predecessor applied binding')
    binding_path = secure_path(path.parent, 'after/' + binding_relative)
    recovery_one_shots(bundle, previous, path)
    pins = {'path': str(path), 'sha256': digest(path), 'treeSha256': tree_digest(path.parent),
            'selectedBundle': bundle, 'bindingSha256': digest(binding_path)}
    if state is not None:
        require(state['oldBundle'] == bundle and state['predecessor'] == pins, 'Continuation predecessor/resume pins changed')
        require(successor_name in (None, state['transitionName']), 'Continuation successor/resume name changed')
        successor_name = state['transitionName']
    if successor_name is not None:
        recovery_successor(root, bundle, previous, pins, successor_name, state)
    return previous, pins


def recovery_unstarted(root, domain, campaign, reconciliation):
    require(isinstance(campaign, str) and len(campaign) <= 64
            and re.fullmatch(CAMPAIGN + r'(?:-retry-[1-9][0-9]*)?', campaign) is not None, 'Invalid qualified Hoodi campaign name')
    require(not secure_path(root, 'campaigns/' + campaign).exists(), 'Named campaign already started; recovery is not admitted')
    evidence = pinned_evidence(reconciliation)
    require(evidence.get('schemaVersion') == 1 and evidence.get('testOnly') is True
            and evidence.get('runId') == root.name and evidence.get('status') == 'VERIFIED'
            and evidence.get('unresolvedOriginals') == [], 'Original signed outcomes are not reconciled')
    named = evidence.get('namedCampaigns', {})
    require(isinstance(named, dict), 'Named campaign reconciliation is missing')
    campaigns = secure_path(root, 'campaigns')
    trees = {}
    names = set()
    if campaigns.exists():
        require(stat.S_ISDIR(campaigns.lstat().st_mode), 'Named campaign owner is not a directory')
        for entry in campaigns.iterdir():
            path = secure_path(root, 'campaigns/' + entry.name)
            require(re.fullmatch(CAMPAIGN + r'(?:-retry-[1-9][0-9]*)?', entry.name) is not None
                    and len(entry.name) <= 64 and stat.S_ISDIR(path.lstat().st_mode)
                    and stat.S_IMODE(path.stat().st_mode) == 0o700, 'Unsafe retained named campaign')
            journal = secure_path(path, 'campaign-state.json')
            require(journal.exists() and stat.S_ISREG(journal.lstat().st_mode)
                    and stat.S_IMODE(journal.stat().st_mode) == 0o600, 'Retained named campaign journal is missing or unsafe')
            value = load(journal)
            require(value.get('schemaVersion') == 3 and value.get('runId') == entry.name
                    and value.get('status') == 'failed_before_t0' and value.get('preflight', {}).get('status') == 'failed',
                    'Retained named campaign is not terminal preflight-failed; preserve its existing owner')
            row = named.get(entry.name, {})
            require(isinstance(row, dict), 'Named campaign reconciliation is malformed')
            sha = row.get('campaignTreeSha256')
            require(row.get('status') == 'VERIFIED' and row.get('unresolvedOriginals') == []
                    and isinstance(sha, str) and re.fullmatch('[0-9a-f]{64}', sha) is not None
                    and tree_digest(path) == sha, 'Retained named campaign is unrecorded, changed or unresolved')
            names.add(entry.name)
            trees[str(path.relative_to(root))] = sha
    require(set(named) == names, 'Named campaign reconciliation inventory differs from retained campaigns')
    app = secure_path(root, 'app-messages')
    if app.exists():
        require(stat.S_ISDIR(app.lstat().st_mode), 'Application owner is not a directory')
        for entry in app.iterdir():
            mode = entry.lstat().st_mode
            artifact = re.fullmatch(r'artifacts(?:-[A-Za-z0-9_-]+)?', entry.name)
            public = re.fullmatch(r'public-[A-Za-z0-9_-]+\.json', entry.name)
            require(artifact and stat.S_ISDIR(mode) or public and stat.S_ISREG(mode),
                    'Application preparation/intent already exists; recovery is not admitted')
            if public:
                value = load(entry)
                require(value.get('schemaVersion') == 1 and value.get('testOnly') is True and value.get('runId') == root.name,
                        'Public application history has a different identity')
    names.add(CAMPAIGN)
    supervisors = secure_path(root, 'supervisors')
    for mode in ('preflight', 'warmup'):
        for path in supervisors.glob(CAMPAIGN + '*-' + mode + '.plist'):
            name = path.name.removesuffix('-' + mode + '.plist')
            require(re.fullmatch(CAMPAIGN + r'(?:-retry-[1-9][0-9]*)?', name) is not None and len(name) <= 64,
                    'Unsafe named one-shot identity')
            names.add(name)
    for mode in ('preflight', 'warmup'):
        for relative in ['supervisors/' + mode + '-one-shot.plist'] + [
                'supervisors/' + name + '-' + mode + '.plist' for name in sorted(names)]:
            path = secure_path(root, relative)
            require(path.exists() and launch_observation(domain, load_plist(path)['Label']) is None,
                    'Historical/named one-shot is loaded or missing; HOLD')
    return trees


def recovery_actors(root, domain, previous, bundle, table):
    actors = {}
    for name in ACTORS:
        label = load_plist(root / 'supervisors' / (name + '.plist'))['Label']
        require(label == previous['actors'][name]['label'], 'Original actor label changed')
        observation = launch_observation(domain, label)
        if observation is None:
            require(name not in ('alice', 'bob'), 'Source actor is not live for pre-stop authentication')
            actors[name] = {'label': label, 'pid': None, 'process': None, 'fenced': True}
            continue
        process = table.get(observation['pid'])
        binary = 'gear' if name in ('alice', 'bob') else 'beefy-relay' if name == 'follower' else 'relayer'
        native = str(Path(bundle['path']) / 'bin' / binary)
        require(process and process['pgid'] == process['pid']
                and (process['command'] == native or process['command'].startswith(native + ' ')),
                'Current supervised actor is not its authenticated native executable/process group')
        actors[name] = {**observation, 'process': process}
    require({entry['pid'] for entry in scoped_processes(table, root, previous['checkpoint'])} ==
            {entry['pid'] for entry in actors.values() if entry['pid'] is not None},
            'Manual/unowned wrapper or native process is not fenced')
    return actors


def continuation_funding(root, config, deployment, budget, timeout=10):
    """Read-only finalized balance/nonce accounting; this helper never funds or re-signs."""
    from eth_account import Account
    from eth_account.typed_transactions import TypedTransaction
    from eth_utils import keccak
    from hexbytes import HexBytes
    roles = load(root / 'hoodi/addresses.json')['roles']
    minimum = budget['minimumBalanceWei']
    require(set(minimum) == {'follower', 'root', 'paid', 'campaign'} and all(isinstance(value, str)
            and re.fullmatch(r'[1-9][0-9]*', value) for value in minimum.values()), 'Missing exact continuation funding budgets')
    execution = execution_state(config, deployment['ethereum'], timeout)
    pin = {'blockHash': execution['finalizedHash'], 'requireCanonical': True}
    def rpc(method, params):
        return execution_rpc(config, method, params, timeout)
    accounts = {}
    for role in minimum:
        address = roles[role]
        balance = int(rpc('eth_getBalance', [address, pin]), 16)
        nonces = {tag: int(rpc('eth_getTransactionCount', [address, pin if tag == 'finalized' else tag]), 16)
                  for tag in ('finalized', 'latest', 'pending')}
        require(balance >= int(minimum[role]), 'Fund and finalize the Hoodi ' + role + ' budget before a new gate')
        require(nonces['finalized'] <= nonces['latest'] <= nonces['pending'], 'Actor nonce observations are incoherent')
        accounts[role] = {'address': address, 'balanceWei': str(balance), 'minimumBalanceWei': minimum[role], 'nonces': nonces}
    follower = load(root / 'follower/state.json')
    require(follower['followerSigner'].lower() == roles['follower'].lower()
            and follower['rootPublisherSigner'].lower() == roles['root'].lower()
            and follower['activeEthereum'] == deployment['ethereum'], 'Original follower/signing lane identity changed')
    lower, upper = (accounts['follower']['nonces'][tag] for tag in ('finalized', 'pending'))
    def authenticate_submission(submission, commitment=None):
        require(isinstance(submission, dict) and all(field in submission for field in
                ('rawTransaction', 'nonce', 'txHash', 'clientAddress')), 'Original signed follower submission is missing; HOLD')
        raw = HexBytes(submission['rawTransaction'])
        transaction = TypedTransaction.from_bytes(raw).as_dict()
        nonce = int(submission['nonce'])
        if commitment is not None and not (lower <= transaction['nonce'] < upper or lower <= nonce < upper):
            return None
        tx_hash = '0x' + keccak(raw).hex()
        require(tx_hash == submission['txHash'] and transaction['nonce'] == nonce and transaction['chainId'] == 560048
                and Account.recover_transaction(raw).lower() == roles['follower'].lower()
                and HexBytes(transaction['to']).hex().removeprefix('0x') == deployment['ethereum']['client'][2:].lower()
                and submission['clientAddress'] == deployment['ethereum']['client'] and transaction['value'] == 0,
                'Original signed follower transaction identity changed')
        if commitment is not None:
            require(commitment.get('txHash') == tx_hash
                    and commitment.get('clientAddress') in (None, deployment['ethereum']['client']),
                    'Commitment substituted its original signed follower submission; HOLD')
        maximum_cost = transaction['gas'] * transaction['maxFeePerGas'] + transaction['value']
        require(int(accounts['follower']['balanceWei']) >= maximum_cost, 'Original follower maximum cost is unfunded')
        observed = rpc('eth_getTransactionByHash', [tx_hash])
        receipt = rpc('eth_getTransactionReceipt', [tx_hash])
        if observed is None:
            require(receipt is None and accounts['follower']['nonces']['pending'] <= nonce,
                    'Original follower nonce is consumed without its transaction; HOLD')
        else:
            require(observed['hash'] == tx_hash and observed['from'].lower() == roles['follower'].lower()
                    and observed['to'].lower() == deployment['ethereum']['client'].lower() and int(observed['nonce'], 16) == nonce,
                    'RPC substituted the original follower transaction')
        return {'txHash': tx_hash, 'nonce': str(nonce), 'maximumCostWei': str(maximum_cost),
                'transactionObserved': observed is not None, 'receiptObserved': receipt is not None}
    submission = follower.get('submission')
    original = authenticate_submission(submission) if submission is not None else None
    owned = {int(original['nonce']): original} if original is not None and lower <= int(original['nonce']) < upper else {}
    if lower < upper:
        commitments = follower.get('commitments', [])
        require(isinstance(commitments, list), 'Original follower commitment inventory is missing; HOLD')
        for entry in commitments:
            require(isinstance(entry, dict), 'Original follower commitment is malformed; HOLD')
            evidence = authenticate_submission(entry.get('submission'), entry)
            if evidence is not None:
                nonce = int(evidence['nonce'])
                require(nonce not in owned or owned[nonce]['txHash'] == evidence['txHash'],
                        'Conflicting original follower ownership for nonce ' + str(nonce) + '; HOLD')
                owned[nonce] = evidence
    for role in ('root', 'paid', 'campaign'):
        require(len(set(accounts[role]['nonces'].values())) == 1, 'Unsettled ' + role + ' nonce; retain its original owner')
    require(len(owned) == upper - lower, 'Follower nonce is unowned; HOLD')
    remaining = [evidence for nonce, evidence in sorted(owned.items()) if original is None or evidence['txHash'] != original['txHash']]
    return {'execution': execution, 'accounts': accounts, 'originalFollowerSubmission': original,
            'unsettledFollowerSubmissions': remaining, 'fundingPerformedByTransition': False}


def recovery_root_continuity(root, config, deployment, state, execution, follower, timeout=10):
    """Read-only root proof, not checkpoint/BEEFY progress or token-campaign acceptance."""
    from eth_utils import keccak
    from substrateinterface import SubstrateInterface
    require(recovery_number(state['transitionName']) > 0, 'Idle roots apply only to a separately journaled continuation')
    continuation_authorization(root, state['continuationAuthorization'])
    target, cursor = state['rootScanTarget'], follower['rootScan']
    require(cursor['block'] >= target['height'], 'Root scanner has not reached the pinned finalized target')
    roots = follower['roots']
    require(isinstance(roots, dict) and roots, 'Missing retained root inventory')
    snapshot = secure_path(root, 'bundle-transitions/' + state['transitionName'] + '/before-runtime')
    original = load(secure_path(snapshot, 'follower/state.json'))['roots']
    require(original and all(roots.get(key) == value for key, value in original.items()), 'Original root inventory changed')
    directory = secure_path(root, 'follower/root-publications')
    inventory = {secure_path(directory, key + '.json') for key in roots}
    require(set(directory.iterdir()) == inventory, 'Orphan or unfinished root publication; HOLD')
    ethereum = deployment['ethereum']
    expected_identity = {'sourceGenesis': deployment['anchor']['sourceGenesis'], 'sourceDomain': ethereum['sourceDomain'],
                         'bridgeDomain': ethereum['bridgeDomain'], 'destinationChainId': 560048,
                         'destinationQueue': ethereum['queue']}
    publisher = follower['rootPublisherSigner']
    require(publisher.lower() == load(root / 'hoodi/addresses.json')['roles']['root'].lower(), 'Root publisher identity changed')
    records = []
    for key, item in sorted(roots.items(), key=lambda entry: entry[1]['block']):
        require(key == str(item['block']) + '-' + item['queueRoot'][2:]
                and item['kind'] in ('merkleRoot', 'emptyProgress') and item['status'] == 'accepted'
                and item['block'] <= cursor['block'] and re.fullmatch('0x[0-9a-f]{64}', item['queueRoot']) is not None,
                'Root publication is unresolved or malformed; HOLD')
        path = secure_path(directory, key + '.json')
        require(Path(item['publication']) == path, 'Root publication escaped its original inventory')
        publication = load(path)
        require(publication['schemaVersion'] == 3 and publication['status'] == 'accepted'
                and publication['finalityStatus'] == 'finalized' and publication['publicationReceipt']['finalized'] is True,
                'Root publication is mined/pending/failed, not finalized; HOLD')
        if key in original:
            original_path = secure_path(snapshot, 'follower/root-publications/' + key + '.json')
            require(digest(path) == digest(original_path), 'Original root publication bytes changed')
        proof = publication['proof']
        require(publication['sourceBlock'] == proof['sourceBlock'] == item['block']
                and publication['root'] == proof['queueRoot'] == item['queueRoot']
                and proof['sourceHash'] == item['blockHash'] and proof['queueId'] == item['queueId']
                and publication['kind'] == item['kind'] and publication['sourceIdentity'] == expected_identity
                and publication['sender'] == publisher and publication['acceptedAnchorClient'] == ethereum['client'],
                'Original root publication source/deployment identity changed')
        records.append((item, publication, digest(path)))
    require(execution['queueBlock'] >= state['executionBefore']['queueBlock']
            and execution['queueBlock'] == max(item['block'] for item, _, _ in records), 'Finalized queue inventory regressed or is incomplete')
    root_advanced = any(item['kind'] == 'merkleRoot' and item['block'] > state['executionBefore']['queueBlock']
                        for item, _, _ in records)
    require(execution['queueBlock'] == state['executionBefore']['queueBlock'] or root_advanced,
            'New queue height is not an authenticated newer message root')
    source_checks = []
    for endpoint in (config['source']['aliceRpc'], config['source']['bobRpc']):
        api = SubstrateInterface(url=endpoint, ws_options={'timeout': timeout})
        try:
            require(api.get_block_hash(0) == expected_identity['sourceGenesis']
                    and api.get_block_number(api.get_chain_finalised_head()) >= cursor['block']
                    and api.get_block_hash(target['height']) == target['hash']
                    and api.get_block_hash(cursor['block']) == cursor['blockHash'], 'Root scanner target/cursor is not canonical finalized history')
            for item, _, _ in records:
                block_hash = item['blockHash']
                require(api.get_block_hash(item['block']) == block_hash
                        and api.query('GearEthBridge', 'QueueId', block_hash=block_hash).value == item['queueId']
                        and api.query('GearEthBridge', 'QueueMerkleRoot', block_hash=block_hash).value == item['queueRoot'],
                        'Stored root differs from original source/witness history')
                if item['kind'] == 'merkleRoot':
                    events = [event.value['event']['attributes'] for event in api.get_events(block_hash=block_hash)
                              if event.value['event']['module_id'] == 'GearEthBridge'
                              and event.value['event']['event_id'] == 'QueueMerkleRootChanged']
                    require(events == [{'queue_id': item['queueId'], 'root': item['queueRoot']}], 'Original source root registration event changed')
            source_checks.append({'endpoint': endpoint, 'target': target, 'cursor': cursor})
        finally:
            api.close()
    def rpc(method, params):
        return execution_rpc(config, method, params, timeout)
    require(int(rpc('eth_chainId', []), 16) == 560048
            and rpc('eth_getBlockByNumber', ['0x0', False])['hash'] == config['network']['genesisHash'], 'Root proof is not public Hoodi')
    finalized = rpc('eth_getBlockByNumber', ['finalized', False])
    require(int(finalized['number'], 16) >= execution['finalizedHeight']
            and rpc('eth_getBlockByNumber', [hex(execution['finalizedHeight']), False])['hash'] == execution['finalizedHash'],
            'Root proof destination pin is not canonical finalized history')
    pin = {'blockHash': execution['finalizedHash'], 'requireCanonical': True}
    nonces = [int(rpc('eth_getTransactionCount', [publisher, block]), 16) for block in ('latest', 'pending', pin)]
    require(nonces[0] == nonces[1] == nonces[2], 'Root publisher has an unresolved/unfinalized nonce; HOLD')
    authenticated = []
    for item, publication, publication_sha in records:
        signed = bytes.fromhex(publication['rawTransaction'][2:])
        tx_hash = publication['txHash']
        require(signed and '0x' + keccak(signed).hex() == tx_hash, 'Original signed root hash changed')
        transaction, receipt = rpc('eth_getTransactionByHash', [tx_hash]), rpc('eth_getTransactionReceipt', [tx_hash])
        saved = publication['publicationReceipt']
        block = saved['block']
        require(transaction is not None and receipt is not None and receipt['transactionHash'] == transaction['hash'] == tx_hash
                and receipt['from'].lower() == transaction['from'].lower() == publisher.lower()
                and receipt['to'].lower() == transaction['to'].lower() == ethereum['queue'].lower()
                and int(transaction['nonce'], 16) == int(publication['nonce']) and int(transaction['value'], 16) == 0
                and int(receipt['status'], 16) == 1 and int(receipt['blockNumber'], 16) == int(transaction['blockNumber'], 16) == block
                and receipt['blockHash'] == transaction['blockHash'] == saved['blockHash']
                and block <= saved['finalizedBlock'] <= execution['finalizedHeight']
                and rpc('eth_getBlockByNumber', [hex(block), False])['hash'] == saved['blockHash']
                and rpc('eth_getBlockByNumber', [hex(saved['finalizedBlock']), False])['hash'] == saved['finalizedBlockHash'],
                'Original root receipt is failed, nonfinal, orphaned or substituted; HOLD')
        block_word = item['block'].to_bytes(32, 'big')
        root_word = bytes.fromhex(item['queueRoot'][2:])
        proof_bytes = bytes.fromhex(publication['queueProof'][2:])
        signature = 'submitMerkleRoot(uint256,bytes32,bytes)' if item['kind'] == 'merkleRoot' else 'submitEmptyQueueProgress(uint256,bytes)'
        fixed = block_word + (root_word if item['kind'] == 'merkleRoot' else b'')
        calldata = (keccak(text=signature)[:4] + fixed + (len(fixed) + 32).to_bytes(32, 'big')
                    + len(proof_bytes).to_bytes(32, 'big') + proof_bytes + bytes((-len(proof_bytes)) % 32))
        require(transaction['input'].lower() == '0x' + calldata.hex(), 'Original root call/calldata changed')
        root_call = '0x' + (keccak(text='getMerkleRoot(uint256)')[:4] + block_word).hex()
        stored = rpc('eth_call', [{'to': ethereum['queue'], 'data': root_call}, pin])
        require(stored == item['queueRoot'], 'Finalized destination stored root changed')
        topic = '0x' + keccak(text='MerkleRoot(uint256,bytes32)' if item['kind'] == 'merkleRoot' else 'EmptyQueueProgress(uint256)').hex()
        topics = [topic] + (['0x' + block_word.hex()] if item['kind'] == 'emptyProgress' else [])
        data = '0x' + ((block_word + root_word).hex() if item['kind'] == 'merkleRoot' else '')
        logs = [log for log in receipt['logs'] if log['address'].lower() == ethereum['queue'].lower()
                and log['topics'] == topics and log['data'] == data and log.get('removed', False) is False
                and log['transactionHash'] == tx_hash and log['blockHash'] == saved['blockHash']
                and int(log['blockNumber'], 16) == block]
        require(len(logs) == 1, 'Original finalized queue root event missing or ambiguous')
        authenticated.append({'sourceBlock': item['block'], 'sourceHash': item['blockHash'], 'queueId': item['queueId'],
                              'root': item['queueRoot'], 'kind': item['kind'], 'publicationSha256': publication_sha,
                              'txHash': tx_hash, 'receipt': saved, 'logIndex': int(logs[0]['logIndex'], 16)})
    return {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'transitionName': state['transitionName'],
            'criterion': 'finalized-new-root' if root_advanced else 'verified-idle-root-continuity',
            'authorization': state['continuationAuthorization'], 'scanTarget': target, 'scanner': cursor,
            'sourceChecks': source_checks, 'execution': execution, 'queueBlockBefore': state['executionBefore']['queueBlock'],
            'queueBlockDelta': execution['queueBlock'] - state['executionBefore']['queueBlock'],
            'rootPublisher': {'address': publisher, 'finalizedNonce': nonces[2]},
            'allPublicationsFinalized': True, 'roots': authenticated}


def recovery_identity(state):
    fields = ('schemaVersion', 'testOnly', 'runId', 'transitionName', 'oldBundle', 'newBundle', 'helperSha256',
              'actors', 'originalProcesses', 'sourceBefore', 'sourceBeforeSha256', 'preparedAtMs', 'predecessor',
              'mutableFiles', 'preservedFiles', 'allowlistedArtifactDiff', 'originalCampaignTreeSha256', 'checkpoint',
              'originalIntentReconciliation', 'embeddedPrograms', 'executionBefore', 'checkpointBefore',
              'campaignsLaunched', 'nativeJournalManuallyEdited', 'fundingPerformedByTransition', 'identitiesReset', 'preservedArtifactTrees',
              'continuationAuthorization', 'fundingBefore', 'historyTrees', 'rootScanTarget')
    return {**{field: state[field] for field in fields},
            'stopGate': {field: state['stopGate'][field] for field in ('name', 'startedAtMs', 'deadlineAtMs')},
            'sourceGate': {field: state['sourceGate'][field] for field in ('name', 'absoluteCapAtMs')},
            'progressGate': {field: state['progressGate'][field] for field in ('name', 'deadlineAtMs')}}


def validate_recovery_history(root, state, record):
    recovery_predecessor(root, state['oldBundle'], state['predecessor'], state)
    continuation_authorization(root, state['continuationAuthorization'])
    history = secure_path(root, 'bundle-transitions')
    require({str(path.relative_to(root)) for path in history.iterdir() if path != record.parent} == set(state['historyTrees']),
            'Historical transition inventory changed')
    for relative, expected in state['historyTrees'].items():
        require(tree_digest(secure_path(root, relative)) == expected, 'Historical transition tree changed: ' + relative)
    for relative, expected in state['preservedArtifactTrees'].items():
        require(tree_digest(secure_path(root, relative)) == expected, 'Preserved application artifact tree changed')


def validate_recovery_record(root, state, record):
    validate_recovery_history(root, state, record)
    frozen_recovery_record(state, record.parent)
    prepared = state['preparedAtMs']
    require(state['stopGate']['deadlineAtMs'] == prepared + 30000
            and state['sourceGate']['absoluteCapAtMs'] == prepared + 150000
            and state['progressGate']['deadlineAtMs'] == prepared + CONTINUATION_PROGRESS_MS, 'Recovery operational deadlines changed')
    source = state['sourceGate']
    if 'startedAtMs' in source:
        require(source['deadlineAtMs'] == min(source['startedAtMs'] + 120000, source['absoluteCapAtMs']),
                'Recovery source gate deadline changed')
    if state['status'] == 'PASS':
        require(all(state[gate]['status'] == 'PASS' for gate in ('stopGate', 'sourceGate', 'progressGate')),
                'Failed continuation gate cannot be labeled PASS')
        path = secure_path(record.parent, 'queue-root-continuity.json')
        binding = state['queueRootContinuity']
        evidence = load(path)
        delta = evidence['execution']['queueBlock'] - state['executionBefore']['queueBlock']
        require(binding['path'] == str(path) and binding['sha256'] == digest(path)
                and evidence == state['appliedObservation']['queueRootEvidence']
                and evidence['schemaVersion'] == 1 and evidence['testOnly'] is True and evidence['runId'] == root.name
                and evidence['transitionName'] == state['transitionName'] and evidence['authorization'] == state['continuationAuthorization']
                and evidence['scanTarget'] == state['rootScanTarget'] and evidence['scanner']['block'] >= state['rootScanTarget']['height']
                and evidence['allPublicationsFinalized'] is True and evidence['roots'] and delta >= 0
                and evidence['queueBlockDelta'] == delta
                and evidence['criterion'] == ('finalized-new-root' if delta > 0 else 'verified-idle-root-continuity'),
                'Completed recovery root-continuity evidence changed or is missing')


def transition(args):
    root = Path(os.environ.get('BEEFY_RUN', '')).absolute()
    require(root.name == RETAINED_RUN and root == root.resolve(strict=True), 'Only the retained canonical Hoodi run is admitted')
    os.umask(0o077)
    config = load(secure_path(root, 'run.json'))
    require(config['schemaVersion'] == 1 and config['runId'] == root.name and config['testOnly'] is True
            and config['network']['chainId'] == 560048, 'Not the retained Hoodi descriptor')
    candidate = args.candidate_bundle.absolute()
    require(re.fullmatch('[0-9a-f]{64}', args.candidate_sha256) is not None, 'Candidate SHA256 must be exact lowercase hex')
    new_manifest = authenticate_bundle(candidate, args.candidate_sha256)
    helper = candidate / 'ops/setup-services.py'
    require(Path(__file__).absolute() == helper and digest(helper) == new_manifest['files']['ops/setup-services.py'],
            'Transition must execute the authenticated candidate sealed helper')
    recovery = recovery_number(args.transition_name) > 0
    verification = qualify_candidate(candidate, new_manifest)

    admission = runpy.run_path(str(candidate / 'ops/run-preflight.py'))
    campaign = admission['qualified_campaign'](candidate, new_manifest['files']['verification.json'])
    admission['app_artifact'](root, campaign, 'eth-to-vara', candidate, new_manifest['files']['verification.json'])
    directory = secure_path(root, 'bundle-transitions/' + args.transition_name)
    record = directory / 'transition.json'
    state, pending_save = transition_journal(record)
    if state is not None:
        require(args.resume, 'Existing transition requires --resume with its exact candidate')
        require(state['runId'] == root.name and state['transitionName'] == args.transition_name
                 and state['newBundle'] == {'path': str(candidate), 'sha256': args.candidate_sha256}, 'Resume identity changed')
        old = Path(state['oldBundle']['path'])
        require(state['helperSha256'] == digest(helper), 'Original transition helper changed')
        require(config['bundle'] in (state['oldBundle'], state['newBundle']), 'Selected bundle is a third value; HOLD')
    else:
        require(not args.resume and not directory.exists(), 'Unfinished transition directory; reconcile it without a new gate')
        old = Path(config['bundle']['path'])
        state = None
    old_binding = state['oldBundle'] if state is not None else config['bundle']
    old_manifest = authenticate_bundle(old, old_binding['sha256'])
    diff = bundle_diff(old_manifest, new_manifest, recovery)
    if recovery:
        qualify_native_changes(old_manifest, new_manifest, verification)
        qualify_application_rebind(root, old, candidate, old_manifest, new_manifest, admission)
    domain = 'gui/' + str(os.getuid())
    with contextlib.ExitStack() as stack:
        # Keep the original admission-lock order. Children do not inherit these short-lived setup locks.
        for relative in ('bundle-transition.lock', 'program-setup.lock', 'service-setup.lock', 'source-chain/supervisor-setup.lock',
                         'forge-final/deployment.lock', 'bounded-campaign.lock'):
            fd = os.open(secure_path(root, relative), os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
            lock = stack.enter_context(os.fdopen(fd, 'a'))
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        previous = predecessor_pins = None
        campaign_trees = {}
        if recovery:
            continuation_authorization(root, verification['continuationAuthorization'])
            expected = continuation_predecessor_pin(root, verification, state, args.predecessor_sha256)
            previous, predecessor_pins = recovery_predecessor(root, old_binding, expected, state, args.transition_name)
            if state is not None:
                validate_recovery_record(root, state, record)
            if state is None or state['status'] != 'PASS':
                campaign_trees = recovery_unstarted(root, domain, campaign, verification['originalIntentReconciliation'])
        if pending_save is not None:
            cas_replace(record, *pending_save)
        deployment = load(root / 'deployment.json')
        stack_manifest = load(root / 'token-stack/token-stack.json')
        if state is None:
            require(not secure_path(root, 'campaigns/' + campaign).exists(), 'Named attempt already exists before its transition')
            for mode in ('preflight', 'warmup'):
                require(launch_observation(domain, load_plist(root / 'supervisors' / (mode + '-one-shot.plist'))['Label']) is None,
                        'Original historical one-shot is still loaded')
            funding_before = continuation_funding(root, config, deployment, verification['continuationFunding']) if recovery else None
            history_trees = {str(path.relative_to(root)): tree_digest(path)
                             for path in secure_path(root, 'bundle-transitions').iterdir()} if recovery else {}
            directory.mkdir(mode=0o700, parents=True)
            fsync_parent(directory)
            before_launch = native_source_state(old / 'bin/beefy-relay', root, config, directory / 'source-before.json', 120)
            old_launch = load(root / 'source-chain/launch-state.json')
            require(before_launch['identity'] == {**old_launch['identity'], 'relayBinarySha256': '0x' + digest(old / 'bin/beefy-relay')},
                    'Deployed source identity differs from the retained runtime/activation/domain pins')
            source_before = authenticate_history(config, load(root / 'anchor.json'),
                previous.get('sourceAfter', previous['sourceBefore']) if recovery else None)
            common_hash = next(iter(source_before.values()))['commonHash']
            embedded = embedded_programs(old, candidate, verification, stack_manifest, stack_manifest['checkpoint'], config, common_hash)
            execution_before = execution_state(config, deployment['ethereum'])
            checkpoint_before = applied_checkpoint(config, stack_manifest['checkpoint'], source_before)
            if recovery:
                prior_execution = previous.get('appliedObservation', {}).get('execution', previous['executionBefore'])
                require(all(execution_before[field] >= prior_execution[field] for field in ('finalizedHeight', 'beefyBlock', 'queueBlock'))
                        and execution_rpc(config, 'eth_getBlockByNumber', [hex(prior_execution['finalizedHeight']), False])['hash']
                            == prior_execution['finalizedHash'], 'Predecessor finalized execution history regressed/changed')
                prior_checkpoint = previous.get('appliedObservation', {}).get('checkpoint', previous['checkpointBefore'])
                require(checkpoint_before['slot'] >= prior_checkpoint['slot'], 'Applied checkpoint history regressed')
            original_campaign_sha = tree_digest(root / 'campaign')
            original_reconciliation = verification['originalIntentReconciliation']
            require(original_reconciliation['status'] == 'VERIFIED' and original_reconciliation['runId'] == root.name
                    and original_reconciliation['campaignTreeSha256'] == original_campaign_sha
                    and original_reconciliation['unresolvedOriginals'] == [], 'Original signed outcomes are not reconciled')
            require(digest(Path(original_reconciliation['path'])) == original_reconciliation['sha256'], 'Original reconciliation evidence changed')
            snapshot = read_snapshot(old / 'bin/beefy-relay', root, config, directory / 'balances-before.json')
            reconciliation = load(Path(original_reconciliation['path']))
            require(snapshot['assets'] == reconciliation['finalizedAssets'], 'Original reconciled economic state changed')
            originals = {relative: secure_path(root, relative).read_bytes() for relative in
                         (previous['mutableFiles'] if recovery else
                          ['run.json', 'source-chain/supervisor-plan.json', 'qualification/components.json']
                          + ['supervisors/' + name + '.plist' for name in ACTORS])}
            plan = json.loads(originals['source-chain/supervisor-plan.json'])
            require(plan['bundleSha256'] == old_binding['sha256'],
                    'Original source supervisor binding changed')
            replacements = {**originals, 'run.json': json_bytes({**config, 'bundle': {'path': str(candidate), 'sha256': args.candidate_sha256}}),
                            'source-chain/supervisor-plan.json': json_bytes({**plan, 'bundleSha256': args.candidate_sha256}),
                            'qualification/components.json': json_bytes({'status': 'VERIFIED', 'bundleSha256': args.candidate_sha256,
                                'checks': verification['checks'], 'verificationSha256': digest(candidate / 'verification.json')})}
            table = process_table()
            actors = recovery_actors(root, domain, previous, old_binding, table) if recovery else {}
            for name in ACTORS:
                relative = 'supervisors/' + name + '.plist'
                plist = plistlib.loads(originals[relative])
                if not recovery:
                    actor = launch_observation(domain, plist['Label'])
                    require(actor is not None and actor['pid'] in table, 'Original supervised actor identity is unavailable')
                    actors[name] = {**actor, 'process': table[actor['pid']]}
                replacements[relative] = replacement_plist(originals[relative], old, candidate)
            preserved = ['source-chain/identity.json', 'source-chain/chain.raw.json', 'source-chain/spec-evidence.json',
                         'source-chain/launch-state.json', 'source-chain/setup/bridge-ready.json', 'source-chain/alice/network-key',
                         'source-chain/bob/network-key', 'anchor.json', 'deployment.json', 'token-stack/token-stack.json',
                         'hoodi/checkpoint-deployment.json', 'hoodi/deployment-finalized.json', 'hoodi/addresses.json',
                         'hoodi/gear-addresses.json', 'hoodi/campaign-inventory.json', 'hoodi/campaign-inventory-finalized.json',
                         'hoodi/funding-complete.json', 'supervisors/preflight-one-shot.plist',
                         'supervisors/warmup-one-shot.plist', 'hoodi/warmup-supervisor-observer.py']
            preserved += ['hoodi/network-gate.json', 'supervisors/service-ports.json']
            preserved += [str(path.relative_to(root)) for pattern in ('hoodi/keys/*.key', 'hoodi/gear-keys/*.suri',
                           'source-chain/setup/queue-bootstrap.*', 'forge-final/*intent*.json') for path in root.glob(pattern)]
            if recovery:
                preserved += [str(path.relative_to(root)) for path in root.glob('app-messages/public-*.json')]
            state = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'transitionName': args.transition_name,
                     'status': 'RUNNING', 'phase': 'prepared', 'oldBundle': config['bundle'],
                     'newBundle': {'path': str(candidate), 'sha256': args.candidate_sha256}, 'helperSha256': digest(helper),
                     'allowlistedArtifactDiff': diff, 'originalCampaignTreeSha256': original_campaign_sha,
                     'preservedFiles': {name: digest(secure_path(root, name)) for name in preserved}, 'actors': actors,
                     'originalProcesses': scoped_processes(table, root, stack_manifest['checkpoint']), 'stopIntents': {}, 'starts': {},
                     'sourceBefore': source_before, 'sourceBeforeSha256': digest(directory / 'source-before.json'),
                     'embeddedPrograms': embedded, 'executionBefore': execution_before, 'checkpointBefore': checkpoint_before,
                     'originalIntentReconciliation': original_reconciliation, 'checkpoint': stack_manifest['checkpoint'],
                     'campaignsLaunched': False, 'nativeJournalManuallyEdited': False,
                     'fundingPerformedByTransition': False, 'identitiesReset': False}
            if recovery:
                state['predecessor'] = predecessor_pins
                state['continuationAuthorization'] = verification['continuationAuthorization']
                state['fundingBefore'] = funding_before
                state['historyTrees'] = history_trees
                target_heads = authenticate_history(config, load(root / 'anchor.json'), source_before)
                target = next(iter(target_heads.values()))
                state['rootScanTarget'] = {'height': target['commonHeight'], 'hash': target['commonHash']}
                state['preservedArtifactTrees'] = {**previous.get('preservedArtifactTrees', {}),
                    **{str(path.relative_to(root)): tree_digest(path)
                       for path in secure_path(root, 'app-messages').iterdir() if path.is_dir()}, **campaign_trees}
            named_replacements = one_shots(root, candidate, state, record, campaign)
            for relative in set(named_replacements) - set(originals):
                require(not secure_path(root, relative).exists(), 'Fresh named one-shot path already exists before fencing')
            replacements.update(named_replacements)
            state['mutableFiles'] = {}
            for relative, data in replacements.items():
                before = originals.get(relative)
                for branch, raw in (('before', before), ('after', data)):
                    if raw is not None:
                        path = directory / branch / relative
                        path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                        with path.open('xb') as stream:
                            stream.write(raw); stream.flush(); os.fsync(stream.fileno())
                        path.chmod(0o400)
                        fsync_parent(path)
                state['mutableFiles'][relative] = {'old': hashlib.sha256(before).hexdigest() if before is not None else None,
                                                  'new': hashlib.sha256(data).hexdigest()}
                if relative.endswith('.plist'):
                    subprocess.run(['/usr/bin/plutil', '-lint', str(directory / 'after' / relative)], check=True, timeout=10)
            if recovery:
                validate_recovery_history(root, state, record)
            # Read-only preparation must not consume the original stop barrier.
            now = clock_ms()
            state.update(preparedAtMs=now, lastWallMs=now,
                         stopGate={'name': 'stop', 'status': 'RUNNING', 'startedAtMs': now, 'deadlineAtMs': now + 30000},
                         sourceGate={'name': 'source', 'status': 'PENDING', 'absoluteCapAtMs': now + 150000},
                         progressGate={'name': 'applied-progress', 'status': 'PENDING',
                                       'deadlineAtMs': now + (CONTINUATION_PROGRESS_MS if recovery else 44 * 60000)})
            if recovery:
                authorization = directory / 'recovery-authorization.json'
                with authorization.open('xb') as stream:
                    stream.write(json_bytes(recovery_identity(state))); stream.flush(); os.fsync(stream.fileno())
                authorization.chmod(0o400)
                fsync_parent(authorization)
                state['recoveryAuthorizationSha256'] = digest(authorization)
            persist(record, state)  # All identities, replacement digests and absolute caps durable BEFORE stop intent.
        require(state['allowlistedArtifactDiff'] == diff, 'Recorded candidate artifact diff changed')
        require(digest(directory / 'source-before.json') == state['sourceBeforeSha256'], 'Pre-stop source evidence changed')
        require(clock_ms() >= state['lastWallMs'], 'Transition clock regressed; HOLD')
        state['lastWallMs'] = clock_ms()
        require(state['stopGate']['status'] != 'FAILED' and state['sourceGate']['status'] != 'FAILED', 'Original timed-out gate is failed')
        require(tree_digest(root / 'campaign') == state['originalCampaignTreeSha256'], 'Original failed campaign changed')
        for relative, expected in state['preservedFiles'].items():
            require(digest(secure_path(root, relative)) == expected, 'Retained identity/history changed: ' + relative)
        for relative, pins in state['mutableFiles'].items():
            path = secure_path(root, relative)
            require((digest(path) if path.exists() else None) in (pins['old'], pins['new']), 'Third mutable digest before resume')
            require(digest(directory / 'after' / relative) == pins['new'], 'Intended replacement snapshot changed')
        if state['status'] == 'PASS':
            for relative, pins in state['mutableFiles'].items():
                require(digest(root / relative) == pins['new'], 'Completed transition binding changed')
            print('Recorded transition PASS; no actor restarted and no campaign launched.')
            return
        stop_barrier(root, domain, state, record, state['stopGate'])
        owner_relative = 'beefy-' + str(deployment['ethereum']['chainId']) + '-' + deployment['ethereum']['queue'][2:].lower() + '.lock'
        ownership = []
        # Do not contend with the follower this transition already started.
        owner_paths = () if 'follower' in state['starts'] else (owner_relative, 'follower/state.lock')
        for relative in owner_paths:
            path = secure_path(root, relative)
            require(stat.S_ISREG(path.lstat().st_mode), 'Original owner lock is missing or nonregular')
            handle = stack.enter_context(path.open('r+b'))
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            ownership.append(handle)
        private_snapshot(root, directory / 'before-runtime', state, record)
        require(not state['unfinishedNativeSaves'], 'Unresolved native save state; existing owner must reconcile, never delete')
        for relative, pins in state['mutableFiles'].items():
            cas_replace(secure_path(root, relative), pins['old'], pins['new'], (directory / 'after' / relative).read_bytes())
        state['phase'] = 'descriptor-and-supervisors-bound'
        persist(record, state)
        if state['sourceGate']['status'] != 'PASS':
            if state['sourceGate']['status'] == 'PENDING':
                started = clock_ms()
                state['sourceGate'].update(status='RUNNING', startedAtMs=started,
                    deadlineAtMs=min(started + 120000, state['sourceGate']['absoluteCapAtMs']))
                persist(record, state)
            try:
                start_actor(root, candidate, domain, 'alice', state, record, state['sourceGate'])
                start_actor(root, candidate, domain, 'bob', state, record, state['sourceGate'])
                source_gate(root, candidate, config, state, record)
            except BaseException as error:
                source = state['sourceGate']
                source.update(status='FAILED' if source['status'] == 'FAILED' or clock_ms() >= source['deadlineAtMs'] else 'HOLD', reason=str(error))
                state['status'] = 'HOLD'
                persist(record, state)
                raise
        else:
            require(digest(directory / 'source-after.json') == state['sourceAfterSha256'], 'Applied source evidence changed on resume')
            for name in ('alice', 'bob'):
                observation = launch_observation(domain, state['actors'][name]['label'])
                require(observation and observation['pid'] == state['starts'][name]['pid'], 'Started source actor changed on resume')
                process = process_table().get(observation['pid'])
                original = state['starts'][name]['process']
                require(process and all(process[field] == original[field] for field in ('started', 'pgid')),
                        'Started source process identity changed on resume')
        gate = state['progressGate']
        require(gate['status'] != 'FAILED', 'Original applied-progress gate failed; no deadline reset')
        gate['status'] = 'RUNNING'
        persist(record, state)
        try:
            for name in ('checkpoint', 'inbound', 'outbound'):
                start_actor(root, candidate, domain, name, state, record, gate)
            # Release both native owners immediately before follower-last, retaining every admission lock.
            for handle in ownership:
                fcntl.flock(handle, fcntl.LOCK_UN)
            start_actor(root, candidate, domain, 'follower', state, record, gate)
            while True:
                remaining(gate, 120)
                heads = authenticate_history(config, load(root / 'anchor.json'), state['sourceBefore'], remaining(gate))
                execution = execution_state(config, deployment['ethereum'], remaining(gate))
                checkpoint = applied_checkpoint(config, state['checkpoint'], heads, remaining(gate))
                follower = load(root / 'follower/state.json')
                state['appliedObservation'] = {'source': heads, 'execution': execution, 'checkpoint': checkpoint,
                                               'followerStatus': follower['follower']['status'], 'atMs': clock_ms()}
                persist(record, state)
                if startup_progress_ready(state, execution, checkpoint, follower):
                    if recovery:
                        state['appliedObservation']['queueRootEvidence'] = recovery_root_continuity(
                            root, config, deployment, state, execution, follower, remaining(gate))
                        persist(record, state)
                    break
                time.sleep(min(30, remaining(gate, 30)))
            log_path = directory / ('history-audit-' + str(time.time_ns()) + '.log')
            with log_path.open('x') as log:
                subprocess.run([str(candidate / 'bin/beefy-relay'), 'tokens-history-audit',
                    '--source-rpc', config['source']['aliceRpc'], '--witness-rpc', config['source']['bobRpc'],
                    '--ethereum-rpc', config['network']['executionWss'], '--deployment-manifest', str(root / 'deployment.json'),
                    '--follower-state', str(root / 'follower/state.json'), '--proof-dir', str(directory / 'history-audit-proofs')],
                    stdout=log, stderr=subprocess.STDOUT, check=True, timeout=remaining(gate, 300))
                log.flush(); os.fsync(log.fileno())
            after = read_snapshot(candidate / 'bin/beefy-relay', root, config,
                                  directory / ('balances-after-' + str(time.time_ns()) + '.json'), remaining(gate, 120))
            require(after['assets'] == load(directory / 'balances-before.json')['assets'], 'Economic state changed during observer transition')
            require(tree_digest(root / 'campaign') == state['originalCampaignTreeSha256'], 'Original campaign no longer byte-identical')
            for relative, expected in state['preservedFiles'].items():
                require(digest(secure_path(root, relative)) == expected, 'Retained identity/history changed during transition: ' + relative)
            for relative, pins in state['mutableFiles'].items():
                require(digest(secure_path(root, relative)) == pins['new'], 'Applied transition binding changed: ' + relative)
            if recovery:
                recovery_unstarted(root, domain, campaign, verification['originalIntentReconciliation'])
                validate_recovery_record(root, state, record)
            require(set(state['starts']) == set(ACTORS), 'Applied service observation is incomplete')
            for name in ACTORS:
                start_actor(root, candidate, domain, name, state, record, gate)
            if recovery:
                heads = authenticate_history(config, load(root / 'anchor.json'), state['sourceBefore'], remaining(gate))
                execution = execution_state(config, deployment['ethereum'], remaining(gate))
                checkpoint = applied_checkpoint(config, state['checkpoint'], heads, remaining(gate))
                follower = load(root / 'follower/state.json')
                require(startup_progress_ready(state, execution, checkpoint, follower), 'Recovery finalized progress/follower health regressed')
                evidence = recovery_root_continuity(root, config, deployment, state, execution, follower, remaining(gate))
                evidence_path = directory / 'queue-root-continuity.json'
                persist(evidence_path, evidence)
                state['queueRootContinuity'] = {'path': str(evidence_path), 'sha256': digest(evidence_path)}
                state['appliedObservation'] = {'source': heads, 'execution': execution, 'checkpoint': checkpoint,
                                               'followerStatus': follower['follower']['status'], 'queueRootEvidence': evidence, 'atMs': clock_ms()}
            remaining(gate)
            gate.update(status='PASS', completedAtMs=clock_ms())
            state.update(status='PASS', phase='applied-progress-and-originals-verified',
                         historyAudit={'path': str(log_path), 'sha256': digest(log_path), 'exitCode': 0}, lastWallMs=clock_ms())
            persist(record, state)
        except BaseException as error:
            gate.update(status='FAILED' if gate['status'] == 'FAILED' or clock_ms() >= gate['deadlineAtMs'] else 'HOLD', reason=str(error))
            state['status'] = 'HOLD'
            persist(record, state)
            persist(root / 'supervisors/bridge-services.json', {'testOnly': True, 'transition': str(record),
                    'status': 'HOLD', 'services': state['starts'], 'appliedObservation': state.get('appliedObservation'),
                    'readiness': 'PIDs are not a PASS; original applied-progress gate is unresolved.'})
            raise
        persist(root / 'supervisors/bridge-services.json', {'testOnly': True, 'transition': str(record), 'status': 'PASS',
                'services': state['starts'], 'appliedObservation': state['appliedObservation'],
                'readiness': ('Observed finalized checkpoint/BEEFY progress and verified idle-root continuity; no newer queue root claimed.'
                              if recovery and state['appliedObservation']['queueRootEvidence']['criterion'] == 'verified-idle-root-continuity'
                              else 'Observed finalized checkpoint/BEEFY/new queue-root progress.'),
                'economicReconciliation': 'Original finalized asset balances unchanged; token qualification has not started.'})
        print('Fenced observer transition PASS; named one-shot plists pinned but NOT launched. Mainnet remains NOT_APPROVED.')


def prepare_runtime_campaign(root, config, bundle, manifest, launch):
    runtime_profile(config['runtimeProfile'], manifest['files']['bin/gear'])
    admission = runpy.run_path(str(bundle / 'ops/run-preflight.py'))
    campaign = admission['qualified_campaign'](bundle, manifest['files']['verification.json'])
    require(launch['identity'].get('runtimeProfile') == config['runtimeProfile'], 'Normal source lacks its immutable profile')
    labels, plists = {}, {}
    for mode in ('preflight', 'warmup'):
        label = 'org.gear.candidate.' + root.name[:8] + '.' + campaign + '.' + mode
        labels[mode] = label
        script = 'run-preflight.py' if mode == 'preflight' else 'warmup-supervisor-observer.py'
        command = [sys.executable, str(bundle / 'ops' / script)]
        if mode == 'preflight':
            command.append(mode)
        command += ['--campaign-name', campaign]
        definition = {'Label': label, 'ProgramArguments': command, 'EnvironmentVariables': {'BEEFY_RUN': str(root), 'PATH': os.environ.get('PATH', '')},
                      'WorkingDirectory': str(root), 'StandardOutPath': str(root / 'supervisors' / (campaign + '-' + mode + '.stdout.log')),
                      'StandardErrorPath': str(root / 'supervisors' / (campaign + '-' + mode + '.stderr.log')),
                      'KeepAlive': False, 'RunAtLoad': True, 'AbandonProcessGroup': False}
        relative = 'supervisors/' + campaign + '-' + mode + '.plist'
        path, data = secure_path(root, relative), plistlib.dumps(definition, sort_keys=False)
        if path.exists():
            require(path.read_bytes() == data, 'Original normal one-shot changed; HOLD')
        else:
            with path.open('xb') as stream:
                stream.write(data); stream.flush(); os.fsync(stream.fileno())
            fsync_parent(path)
        plists[relative] = digest(path)
    binding = {'schemaVersion': 1, 'testOnly': True, 'runId': root.name, 'campaignName': campaign,
               'bundleSha256': config['bundle']['sha256'], 'runtimeProfile': config['runtimeProfile'],
               'runnerSha256': manifest['files']['ops/run-preflight.py'], 'observerSha256': manifest['files']['ops/warmup-supervisor-observer.py'],
               'followerLabel': 'org.gear.candidate.' + root.name[:8] + '.follower', 'preflightLabel': labels['preflight'],
               'warmupLabel': labels['warmup'], 'plists': plists, 'automaticRerun': False, 'launched': False,
               'preservedFiles': {relative: digest(secure_path(root, relative)) for relative in
                   ('source-chain/launch-state.json', 'source-chain/chain.raw.json', 'source-chain/identity.json',
                    'deployment.json', 'token-stack/token-stack.json', 'qualification/components.json')}}
    path = secure_path(root, 'supervisors/normal-campaign-admission.json')
    data = json_bytes(binding)
    if path.exists():
        require(path.read_bytes() == data, 'Normal admission is already immutable; HOLD')
    else:
        with path.open('xb') as stream:
            stream.write(data); stream.flush(); os.fsync(stream.fileno())
        fsync_parent(path)


def qualify_applications(args):
    from run_context import RUN, CONFIG, BUNDLE, MANIFEST
    require(CONFIG.get('runtimeProfile') is not None, 'Post-deployment application admission requires a profiled run')
    admission = runpy.run_path(str(BUNDLE / 'ops/run-preflight.py'))
    campaign = admission['qualified_campaign'](BUNDLE, MANIFEST['files']['verification.json'])
    raw = args.qualification.read_bytes()
    require(hashlib.sha256(raw).hexdigest() == args.qualification_sha256, 'Independently selected application qualification changed')
    selected = json.loads(raw)
    with admission['campaign_lock'](RUN):
        admission['completed_transition'](RUN, CONFIG, BUNDLE, MANIFEST)
        for direction in ('eth-to-vara', 'vara-to-eth'):
            admission['app_artifact'](RUN, campaign, direction, BUNDLE, MANIFEST['files']['verification.json'], qualification=selected)
        path = secure_path(RUN, 'qualification/' + campaign + '-applications.json')
        if path.exists():
            require(path.read_bytes() == raw and not path.stat().st_mode & 0o222, 'Original application qualification is immutable; HOLD')
        else:
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o400)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(raw); stream.flush(); os.fsync(stream.fileno())
            fsync_parent(path)
    print('Post-deployment application qualification admitted; no application or token transaction dispatched.')


def ordinary_main(argv):
    from run_context import RUN as ROOT, CONFIG, BUNDLE, MANIFEST, BIN, artifact, private_text, save

    os.umask(0o077)
    launch = json.loads((ROOT/'source-chain/launch-state.json').read_text())
    require(launch['identity'].get('runtimeProfile') == CONFIG.get('runtimeProfile'), 'Service source/runtime profile differs from immutable admission')
    assert launch['phase'] == 'ready'
    network = json.loads((ROOT/'hoodi/network-gate.json').read_text())
    require(all(network[field] == CONFIG['network'][field] for field in ('executionHttp', 'executionWss', 'beaconHttp')),
            'Service endpoints differ from the retained authenticated descriptor')
    source, witness = launch['aliceRpc'], launch['bobRpc']
    ports_path = ROOT/'supervisors/service-ports.json'
    services = ['checkpoint','follower','inbound','outbound']
    rtk = shutil.which('rtk')
    assert rtk


    def arguments(name, ports):
        checkpoint = json.loads((ROOT/'hoodi/checkpoint-deployment.json').read_text())
        assert checkpoint['sourceGenesis'] == launch['readiness']['genesisHash']
        if name == 'checkpoint':
            return [str(BIN/'relayer'),'eth-gear-core','--program-id',checkpoint['programId'],'--ethereum-beacon-rpc',network['beaconHttp'],'--ethereum-beacon-rpc-timeout','60','--size-batch-multiplier','1','--gear-endpoint',source,'--prometheus-endpoint','127.0.0.1:'+str(ports[name])]
        deployment = json.loads((ROOT/'deployment.json').read_text())
        stack = json.loads((ROOT/'token-stack/token-stack.json').read_text())
        assert stack['configuration']['status'] == 'ready', 'Token configuration is not ready'
        assert deployment['anchor']['sourceGenesis'] == stack['sourceGenesis'] == launch['readiness']['genesisHash']
        if name == 'follower':
            binary = str(artifact('beefy-relay'))
            return [binary,'tokens-follow','--source-rpc',source,'--witness-rpc',witness,'--ethereum-rpc',network['executionWss'],'--wallet',str(ROOT/'hoodi/keys/follower.key'),'--root-wallet',str(ROOT/'hoodi/keys/root.key'),'--deployment-manifest',str(ROOT/'deployment.json'),'--output-dir',str(ROOT/'follower')]
        if name == 'inbound':
            verified = json.loads((ROOT/'hoodi/deployment-finalized.json').read_text())
            creations = [receipt for receipt in verified['receipts'] if (receipt.get('contractAddress') or '').lower() == deployment['ethereum']['receiver'].lower()]
            assert len(creations) == 1 and creations[0]['status'] == '0x1'
            start_block = str(int(creations[0]['blockNumber'], 16))
            return [str(BIN/'relayer'),'eth-gear-tokens','--vft-manager-address',stack['programs']['vftManager']['id'],'--ethereum-rpc',network['executionHttp'],'--ethereum-beacon-rpc',network['beaconHttp'],'--ethereum-beacon-rpc-timeout','60','--gear-endpoint',source,'--storage-path',str(ROOT/'inbound'),'--ethereum-blocks',str(ROOT/'inbound/discovery.json'),'--ethereum-start-block',start_block,'--prometheus-endpoint','127.0.0.1:'+str(ports[name]),'all-token-transfers','--erc20-manager-address',deployment['ethereum']['receiver']]
        environment = json.loads((ROOT/'forge-final/deployment-intent.json').read_text())['environment']
        from_block = 1
        if CONFIG.get('runtimeProfile') is not None:
            from_block = max(launch['identity']['mmrStartBlock'], launch['identity']['domainBindingBlock'])
            require(isinstance(from_block, int) and not isinstance(from_block, bool) and from_block > 0, 'Normal discovery has no authenticated first application block')
        return [str(BIN/'relayer'),'gear-eth-tokens','--from-block',str(from_block),'--ethereum-endpoint',network['executionWss'],'--mq-address',deployment['ethereum']['queue'],'--storage-path',str(ROOT/'outbound/journal'),'--gear-endpoint',source,'--governance-admin',environment['GEAR_GOVERNANCE_ADMIN'],'--governance-pauser',environment['GEAR_GOVERNANCE_PAUSER'],'--confirmations-merkle-root','8','--confirmations-status','8','--prometheus-endpoint','127.0.0.1:'+str(ports[name]),'paid-token-transfers','--bridging-payment-address',stack['programs']['bridgingPayment']['id'],'--web-server-address','127.0.0.1:'+str(ports['web'])]


    if argv[:1] == ['run']:
        assert len(argv) == 2 and argv[1] in services
        name = argv[1]
        ports = json.loads(ports_path.read_text())
        args = arguments(name,ports)
        assert args[0] == str(artifact('beefy-relay' if name == 'follower' else 'relayer'))
        environment = os.environ.copy()
        environment['RUST_LOG'] = 'info'
        if name in ['checkpoint','inbound']:
            environment['GEAR_SURI'] = private_text(ROOT/'hoodi/gear-keys'/(name+'.suri'))
        elif name == 'outbound':
            key = json.loads(private_text(ROOT/'hoodi/keys/paid.key'))
            environment['ETH_FEE_PAYER'] = key['private_key']
            environment['WEB_SERVER_TOKEN'] = private_text(ROOT/'outbound/web-server-token')
        os.execve(args[0],args,environment)

    assert argv in [['checkpoint'],['all']]
    lock = open(ROOT/'service-setup.lock','a')
    fcntl.flock(lock,fcntl.LOCK_EX | fcntl.LOCK_NB)
    selected = ['checkpoint'] if argv[0] == 'checkpoint' else services
    (ROOT/'supervisors').mkdir(mode=0o700,exist_ok=True)
    assert not ports_path.with_suffix('.tmp').exists(), 'Unresolved port reservation write'
    if ports_path.exists():
        ports = json.loads(ports_path.read_text())
    else:
        ports = CONFIG['services']['ports']
        sockets = []
        try:
            for port in ports.values():
                sock = socket.socket()
                sockets.append(sock)
                sock.bind(('127.0.0.1',port))
        finally:
            for sock in sockets:
                sock.close()
        save(ports_path,ports)

    for name in selected:
        (ROOT/name).mkdir(mode=0o700,exist_ok=True)
        arguments(name,ports)
    if argv == ['all'] and CONFIG.get('runtimeProfile') is not None:
        prepare_runtime_campaign(ROOT, CONFIG, BUNDLE, MANIFEST, launch)
    if 'outbound' in selected:
        token = ROOT/'outbound/web-server-token'
        if not token.exists():
            with open(token,'x') as f:
                f.write(secrets.token_urlsafe(32)+'\n')
                f.flush()
                os.fsync(f.fileno())
        private_text(token)

    status = {}
    for name in selected:
        label = 'org.gear.candidate.'+ROOT.name[:8]+'.'+name
        domain = 'gui/'+str(os.getuid())
        path = ROOT/'supervisors'/(name+'.plist')
        config = {'Label':label,'ProgramArguments':[sys.executable,str(Path(__file__).resolve()),'run',name],'EnvironmentVariables':{'BEEFY_RUN':str(ROOT),'PATH':str(Path.home()/'.foundry/bin')+':'+str(Path.home()/'.cargo/bin')+':/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin'},'WorkingDirectory':str(ROOT/name),'StandardOutPath':str(ROOT/name/'stdout.log'),'StandardErrorPath':str(ROOT/name/'stderr.log'),'KeepAlive':True,'ThrottleInterval':15,'RunAtLoad':True}
        data = plistlib.dumps(config,sort_keys=False)
        if path.exists():
            assert path.read_bytes() == data, 'Supervisor definition changed: reconcile before replacement'
        else:
            with open(path,'xb') as f:
                f.write(data)
                f.flush()
                os.fsync(f.fileno())
        target = domain+'/'+label
        check = subprocess.run([rtk,'proxy','launchctl','print',target],capture_output=True,text=True)
        if check.returncode:
            created = subprocess.run([rtk,'proxy','launchctl','bootstrap',domain,str(path)],capture_output=True,text=True)
            assert created.returncode == 0, 'Could not bootstrap '+label+': '+created.stderr
        check = subprocess.run([rtk,'proxy','launchctl','print',target],capture_output=True,text=True)
        assert check.returncode == 0
        state = re.search(r'^\s*state = (.+)$',check.stdout,re.M)
        pid = re.search(r'^\s*pid = ([0-9]+)$',check.stdout,re.M)
        status[name] = {'label':label,'supervisorState':state[1] if state else 'not reported','pid':int(pid[1]) if pid else None,'plist':str(path)}
        print(name,json.dumps(status[name]),flush=True)
    save(ROOT/'supervisors/bridge-services.json',{'testOnly':True,'services':status,'ports':ports,'readiness':'Process launch is not data-plane readiness; verify actor journals and actual transactions.'})

def main(argv=None):
    require(__debug__, 'Private service safety checks require non-optimized Python')
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest='command', required=True)
    for name in ('checkpoint', 'all'):
        commands.add_parser(name)
    run_parser = commands.add_parser('run')
    run_parser.add_argument('actor', choices=('checkpoint', 'follower', 'inbound', 'outbound'))
    transition_parser = commands.add_parser('transition', help='Explicit fenced same-lane observer bundle transition; does not launch campaigns')
    transition_parser.add_argument('--candidate-bundle', type=Path, required=True)
    transition_parser.add_argument('--candidate-sha256', required=True)
    transition_parser.add_argument('--transition-name', required=True)
    transition_parser.add_argument('--predecessor-sha256', help='Current bound predecessor hash; permits reuse of the same qualified bundle')
    transition_parser.add_argument('--resume', action='store_true')
    apps_parser = commands.add_parser('applications', help='Pin owned post-deployment SDK evidence without changing the runtime bundle')
    apps_parser.add_argument('--qualification', type=Path, required=True)
    apps_parser.add_argument('--qualification-sha256', required=True)
    args = parser.parse_args(argv)
    # Deliberately dispatch BEFORE run_context: only this mode authenticates both old and candidate tooling.
    if args.command == 'transition':
        transition(args)
    elif args.command == 'applications':
        qualify_applications(args)
    else:
        ordinary_main(argv)


if __name__ == '__main__':
    main()
