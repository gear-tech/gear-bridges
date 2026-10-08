#!/usr/bin/env python3
"""Observe the named native warmup and perform its exactly-once supervised restart."""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import subprocess
import sys
import time


def supervisor(target, timeout):
    result = subprocess.run(['/bin/launchctl', 'print', target], capture_output=True, text=True,
                            check=True, timeout=timeout)
    match = re.search(r'^\s*pid = ([0-9]+)$', result.stdout, re.M)
    if not match:
        raise RuntimeError('Follower has no supervised process')
    return int(match[1])


def restart(root, path, proof, target, save, require):
    raw = path.read_bytes()
    journal = json.loads(raw)
    evidence = journal['warmup']['evidence']
    gate = evidence.get('restartGate')
    if not gate or proof.exists():
        return
    now = int(time.time() * 1000)
    require(journal['warmup']['status'] == 'running' and gate['atMs'] >= evidence['startedAtMs'] + 30 * 60000
            and gate['atMs'] <= now < gate['deadlineAtMs'], 'Original native restart gate is not active')
    follower = json.loads((root / 'follower/state.json').read_text())
    require(follower['startupSequence'] == gate['beforeStartupSequence'], 'Unplanned prior restart; HOLD')
    before = supervisor(target, min(10, (gate['deadlineAtMs'] - now) / 1000))
    record = {'phase': 'restart-intent-recorded', 'testOnly': True, 'target': target, 'gate': gate,
              'nativeJournalSha256': hashlib.sha256(raw).hexdigest(), 'beforePid': before,
              'beforeStartupSequence': follower['startupSequence'], 'requestedAtMs': now,
              'command': ['/bin/launchctl', 'kickstart', '-k', target]}
    # Persist intent before dispatch. An uncertain command never grants a second restart.
    save(proof, record)
    latest = json.loads(path.read_text())
    require(latest['warmup']['status'] == 'running' and latest['warmup']['evidence']['restartGate'] == gate,
            'Native restart gate changed before dispatch')
    record['dispatchAtMs'] = int(time.time() * 1000)
    remaining = (gate['deadlineAtMs'] - record['dispatchAtMs']) / 1000
    require(remaining > 0, 'Original restart deadline expired before dispatch')
    save(proof, record)
    result = subprocess.run(record['command'], capture_output=True, text=True, timeout=min(10, remaining))
    record.update(phase='supervisor-command-completed', exitCode=result.returncode, stdout=result.stdout,
                  stderr=result.stderr, commandCompletedAtMs=int(time.time() * 1000))
    save(proof, record)
    require(result.returncode == 0, 'Real supervisor restart failed; preserve its native deadline')


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--campaign-name', required=True)
    args = parser.parse_args(argv)
    if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]{0,63}', args.campaign_name):
        raise RuntimeError('Invalid campaign name')
    from run_context import RUN as root, BUNDLE, CONFIG, MANIFEST, save, digest, require
    runner = BUNDLE / 'ops/run-preflight.py'
    admission = runpy.run_path(str(runner))
    require(args.campaign_name == admission['qualified_campaign'](BUNDLE, MANIFEST['files']['verification.json']),
            'Only the qualified named warmup is admitted')
    campaign = admission['contained'](root, 'campaigns/' + args.campaign_name)
    binding_path = admission['observer_binding_path'](root, CONFIG)
    binding = json.loads(binding_path.read_text())
    require(binding['bundleSha256'] == CONFIG['bundle']['sha256'] and binding['campaignName'] == args.campaign_name
            and binding['observerSha256'] == digest(Path(__file__)) and binding['runnerSha256'] == digest(runner),
            'Named observer/runner binding changed')
    for relative, expected in binding['plists'].items():
        require(digest(root / relative) == expected, 'Named one-shot supervisor changed')
    domain = 'gui/' + str(os.getuid()) + '/'
    require(supervisor(domain + binding['warmupLabel'], 10) == os.getpid(), 'Standalone warmup is not supervisor qualification')
    os.umask(0o077)
    with admission['campaign_lock'](root) as campaign_lock:
        campaign = admission['campaign_directory'](root, args.campaign_name, 'warmup')
        lock_path = admission['contained'](root, 'warmup-supervisor-observer-' + args.campaign_name + '.lock')
        fd = os.open(lock_path, os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        with os.fdopen(fd, 'a') as observer_lock:
            fcntl.flock(observer_lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            proof = campaign / 'warmup-supervisor-restart.json'
            invocation = campaign / 'warmup-supervisor-invocation.json'
            require(not proof.exists() and not invocation.exists(), 'Existing observer intent; reconcile, never rerun')
            save(invocation, {'testOnly': True, 'campaignName': args.campaign_name, 'mode': 'warmup',
                              'startedAtMs': int(time.time() * 1000), 'runner': str(runner), 'automaticRerun': False,
                              'observerSha256': digest(Path(__file__)), 'pid': os.getpid(), 'bindingSha256': digest(binding_path)})
            target = domain + binding['followerLabel']
            environment = {key: os.environ[key] for key in ('PATH', 'HOME') if key in os.environ}
            environment.update(BEEFY_RUN=str(root), BEEFY_CAMPAIGN_LOCK_FD=str(campaign_lock.fileno()))
            # Keep the launchd process group, and inherit the same open-file lock (no second acquisition).
            process = subprocess.Popen([sys.executable, str(runner), 'warmup', '--campaign-name', args.campaign_name],
                                       cwd=root, env=environment, pass_fds=(campaign_lock.fileno(), observer_lock.fileno()))
            try:
                while process.poll() is None:
                    restart(root, campaign / 'campaign-state.json', proof, target, save, require)
                    time.sleep(30)
                code = process.wait()
                require(code == 0, 'Native warmup failed; retain its original journal')
                record = json.loads(proof.read_text())
                journal = json.loads((campaign / 'campaign-state.json').read_text())
                observed = journal['warmup']['evidence']['followerRestart']
                after = supervisor(target, 10)
                require(journal['warmup']['status'] == 'passed' and record['exitCode'] == 0 and after != record['beforePid'],
                        'Supervised PID restart and native warmup success are not both proved')
                require(observed['caughtUp'] is True and observed['beforeStartupSequence'] == record['beforeStartupSequence']
                        and observed['afterStartupSequence'] > record['beforeStartupSequence'], 'Native restart did not retain/catch up history')
                record.update(phase='native-warmup-and-real-restart-verified', afterPid=after,
                              nativeObservation=observed, verifiedAtMs=int(time.time() * 1000))
                save(proof, record)
                value = json.loads(invocation.read_text())
                value.update(exitCode=0, completedAtMs=int(time.time() * 1000))
                save(invocation, value)
            except BaseException:
                if process.poll() is None:
                    process.terminate()
                    process.wait()
                raise


if __name__ == '__main__':
    if not __debug__:
        raise SystemExit('Operational safety checks require non-optimized Python')
    main()
