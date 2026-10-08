#!/usr/bin/env python3
"""Run the existing hardened CLIs against this run's authenticated native source."""
import fcntl
import json
import os
import re
import secrets
import shutil
import subprocess
import sys
from pathlib import Path

from run_context import RUN as ROOT, OPS, BIN, CONFIG, request, save, private_text

os.umask(0o077)
lock = open(ROOT/'program-setup.lock', 'a')
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
assert len(sys.argv) == 2 and sys.argv[1] in ['prepare','anchor','configure']
stage = sys.argv[1]
launch = json.loads((ROOT/'source-chain/launch-state.json').read_text())
assert launch['phase'] == 'ready'
source = launch['aliceRpc']
witness = launch['bobRpc']
genesis = launch['readiness']['genesisHash']
for endpoint in [source,witness]:
    assert endpoint.startswith('ws://127.0.0.1:')
    result = request(endpoint.replace('ws://','http://',1), {'jsonrpc':'2.0','id':1,'method':'chain_getBlockHash','params':[0]})
    assert result.get('result') == genesis, 'Source genesis differs from readiness pin'
network = json.loads((ROOT/'hoodi/network-gate.json').read_text())
roles = json.loads((ROOT/'hoodi/gear-addresses.json').read_text())['roles']
rtk = shutil.which('rtk')
assert rtk


def phrase(role):
    path = ROOT/'hoodi/gear-keys'/(role+'.suri')
    return private_text(path)


def execute(label, args, secret=None, environment=None):
    env = os.environ.copy()
    env.update(environment or {})
    result = subprocess.run([rtk,'proxy',*map(str,args)],cwd=ROOT,env=env,capture_output=True,text=True)
    stdout, stderr = result.stdout, result.stderr
    if secret:
        stdout, stderr = stdout.replace(secret,'[REDACTED]'), stderr.replace(secret,'[REDACTED]')
    log = ROOT/'hoodi'/(label+'.log')
    with open(log,'a') as f:
        f.write(stdout+'\n'+stderr+'\nexitCode='+str(result.returncode)+'\n')
        f.flush()
        os.fsync(f.fileno())
    print(label,'exitCode='+str(result.returncode),'log='+str(log),flush=True)
    if result.returncode:
        raise SystemExit('Operation failed; preserve its intent and reconcile '+str(log))
    return stdout


if stage == 'prepare':
    trust = {field: CONFIG['network'][field] for field in ('trustedBootstrapRoot', 'beaconGenesisValidatorsRoot')}
    assert all(network.get(field) == value for field, value in trust.items()), 'Checkpoint trust pins changed since run preparation'
    marker = ROOT/'hoodi/checkpoint-deployment.json'
    if not marker.exists():
        intent = ROOT/'hoodi/checkpoint-intent.json'
        assert not intent.exists() and not intent.with_suffix('.tmp').exists(), 'Checkpoint intent already exists: reconcile the original operation before any retry'
        salt = secrets.token_hex(32)
        save(intent,{'sourceGenesis':genesis,'sourceRpc':source,'account':roles['checkpoint'],'salt':salt,'status':'intent-recorded','testOnly':True,**trust})
        key = phrase('checkpoint')
        output = execute('checkpoint-deployment',[BIN/'checkpoints-tool','--gear-endpoint',source,'--ethereum-beacon-rpc',network['beaconHttp'],'--ethereum-beacon-rpc-timeout','60','--salt',salt,'--trusted-bootstrap-root',trust['trustedBootstrapRoot'],'--trusted-genesis-validators-root',trust['beaconGenesisValidatorsRoot']],key,{'GEAR_SURI':key})
        program = re.search(r'program_id = (0x[0-9a-fA-F]{64})',output)
        code = re.search(r'Using code_id = (0x[0-9a-fA-F]{64})',output)
        checkpoint = re.search(r'checkpoint slot = ([0-9]+), hash = ([0-9a-fA-F]{64})',output)
        period = re.search(r'finality_update slot = ([0-9]+), period = ([0-9]+)',output)
        assert program and code and checkpoint and period, 'Missing deployment output; reconcile original intent, never redeploy blindly'
        save(marker,{'sourceGenesis':genesis,'sourceRpc':source,'beaconRpc':network['beaconHttp'],'programId':program[1].lower(),'codeId':code[1].lower(),'checkpointSlot':int(checkpoint[1]),'checkpointHash':'0x'+checkpoint[2].lower(),'period':int(period[2]),'finalityUpdateSlotAtInitialization':int(period[1]),'deploymentOutput':'hoodi/checkpoint-deployment.log','originalIntent':'hoodi/checkpoint-intent.json','testOnly':True,'qualification':'NOT ESTABLISHED',**trust})
    checkpoint = json.loads(marker.read_text())
    assert checkpoint['sourceGenesis'] == genesis and checkpoint['sourceRpc'] == source
    assert all(checkpoint.get(field) == value for field, value in trust.items()), 'Checkpoint deployment lacks the original approved trust pins; preserve and reconcile it, never redeploy blindly'
    execute('checkpoint-supervisor',[sys.executable,OPS/'setup-services.py','checkpoint'])
    execute('token-programs',[BIN/'beefy-relay','tokens','--source-rpc',source,'--expected-genesis',genesis,'--gear-suri-file',ROOT/'hoodi/gear-keys/governance.suri','--checkpoint',checkpoint['programId'],'--checkpoint-slot',str(checkpoint['checkpointSlot']),'--checkpoint-hash',checkpoint['checkpointHash'],'--output-dir',ROOT/'token-stack'])
    stack = json.loads((ROOT/'token-stack/token-stack.json').read_text())
    assert stack['sourceGenesis'] == genesis and stack['checkpoint'] == checkpoint['programId']
    print('Fresh token programs prepared against real Hoodi checkpoint '+checkpoint['programId'],flush=True)
elif stage == 'anchor':
    path = ROOT/'anchor.json'
    assert not path.exists(), 'Anchor already pinned: inspect deployment intent before replacement'
    output = execute('source-anchor',[BIN/'beefy-relay','tokens-anchor','--source-rpc',source,'--witness-rpc',witness])
    anchor = json.loads(output)
    assert anchor['sourceGenesis'] == genesis
    save(path,anchor)
    print('Fresh independently witnessed source anchor pinned.',flush=True)
else:
    execute('token-configuration',[BIN/'beefy-relay','tokens-configure','--source-rpc',source,'--witness-rpc',witness,'--expected-genesis',genesis,'--gear-suri-file',ROOT/'hoodi/gear-keys/governance.suri','--ethereum-rpc',network['executionWss'],'--wallet',ROOT/'hoodi/keys/follower.key','--deployment-manifest',ROOT/'deployment.json','--token-stack',ROOT/'token-stack/token-stack.json'])
    print('Fresh token configuration verified by the hardened CLI.',flush=True)
