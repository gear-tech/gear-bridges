#!/usr/bin/env python3
"""Reconcile original Foundry hashes, then pin the authenticated deployment manifest."""
import fcntl
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from eth_utils import keccak

import run_context as f
from run_context import RUN as ROOT, artifact
network = json.loads((ROOT/'hoodi/network-gate.json').read_text())
f.RPC = network['executionHttp']
os.umask(0o077)
lock = open(ROOT/'deployment-finality.lock','a')
fcntl.flock(lock,fcntl.LOCK_EX | fcntl.LOCK_NB)
identity = json.loads((ROOT/'source-chain/identity.json').read_text())
intent = json.loads((ROOT/'forge-final/deployment-intent.json').read_text())
launch = json.loads((ROOT/'source-chain/launch-state.json').read_text())
broadcast_path = ROOT/'forge-final/broadcast/BeefyTokens.s.sol/560048/run-latest.json'
broadcast = json.loads(broadcast_path.read_text())
marker = ROOT/'hoodi/deployment-finalized.json'
assert not marker.with_suffix('.tmp').exists(), 'Unresolved finality write'
assert broadcast['chain'] == intent['chainId'] == 560048
assert int(f.rpc('eth_chainId',[]),16) == 560048
assert f.rpc('eth_getBlockByNumber',['0x0',False])['hash'] == network['genesisHash']
owner = identity['deployerAddress']
transactions = broadcast['transactions']
assert transactions and all(t['hash'] for t in transactions)
assert len({t['hash'].lower() for t in transactions}) == len(transactions)
nonces = [int(t['transaction']['nonce'],16) for t in transactions]
assert nonces == list(range(len(nonces))) and intent['reservedNonce'] == 0
assert all(t['transaction']['from'].lower() == owner.lower() and int(t['transaction']['chainId'],16) == 560048 for t in transactions)
bindings = {name:broadcast['returns'][name+'Address']['value'] for name in ['client','verifier','queue','manager']}
assert bindings['queue'].lower() == identity['destinationQueue'].lower() == intent['predictedQueue'].lower()
assert transactions[12]['contractAddress'].lower() == bindings['queue'].lower()
records_by_nonce = {int(t['transaction']['nonce'],16):t for t in transactions}

if not marker.exists():
    deadline = time.monotonic()+1800
    while True:
        finalized = f.rpc('eth_getBlockByNumber',['finalized',False])
        receipts = []
        associations = {}
        complete = True
        for original_row in transactions:
            h = original_row['hash']
            receipt = f.rpc('eth_getTransactionReceipt',[h])
            if receipt is None:
                complete = False
                continue
            actual = f.rpc('eth_getTransactionByHash',[h])
            assert actual is not None and receipt['status'] == '0x1', 'Original deployment transaction failed: hold, never replay'
            nonce = int(actual['nonce'],16)
            assert nonce in records_by_nonce and nonce not in associations, 'Transaction identities are not bijective'
            record = records_by_nonce[nonce]
            expected = record['transaction']
            associations[nonce] = {'hash':h,'contract':record['contractName'],'originalRowNonce':int(original_row['transaction']['nonce'],16)}
            assert receipt['transactionHash'].lower() == actual['hash'].lower() == h.lower()
            assert actual['from'].lower() == receipt['from'].lower() == owner.lower()
            assert int(actual['nonce'],16) == int(expected['nonce'],16)
            assert (actual['to'] or '').lower() == (expected['to'] or '').lower() == (receipt['to'] or '').lower()
            assert actual['input'].lower() == expected['input'].lower() and int(actual['value'],16) == int(expected['value'],16)
            assert f.rpc('eth_getBlockByNumber',[receipt['blockNumber'],False])['hash'] == receipt['blockHash'] == actual['blockHash'], 'Noncanonical deployment receipt'
            if expected['to'] is None:
                assert receipt['contractAddress'].lower() == record['contractAddress'].lower()
            receipts.append(receipt)
            complete &= int(receipt['blockNumber'],16) <= int(finalized['number'],16)
        if complete:
            assert set(associations) == set(nonces), 'Original deployment transaction set is incomplete'
            break
        assert time.monotonic() < deadline, 'Original deployment not finalized: preserve broadcast and nonce reservation'
        time.sleep(60)
    expected_nonce = max(nonces)+1
    assert all(int(f.rpc('eth_getTransactionCount',[owner,tag]),16) == expected_nonce for tag in ['latest','pending','finalized']), 'Unexpected deployer nonce after deployment'
    codes = {}
    for receipt in receipts:
        address = receipt['contractAddress']
        if address:
            code = f.rpc('eth_getCode',[address,finalized['number']])
            assert code != '0x', 'Created contract missing from finalized state'
            codes[address] = '0x'+keccak(hexstr=code).hex()
    f.save(marker,{'phase':'finalized','chainId':560048,'testOnly':True,'deployer':owner,'expectedNextDeployerNonce':expected_nonce,'deploymentBlock':max(int(r['blockNumber'],16) for r in receipts),'finalizedBlock':int(finalized['number'],16),'finalizedHash':finalized['hash'],'bindings':bindings,'receipts':receipts,'runtimeCodeHashes':codes,'originalBroadcast':str(broadcast_path),'verifiedTransactionsByNonce':associations,'hashRowOrdering':'Original Forge rows retained; hashes matched bijectively by canonical sender, nonce, destination, input, value and created address'})
sealed = json.loads(marker.read_text())
assert sealed['bindings'] == bindings and sealed['phase'] == 'finalized'
assert f.rpc('eth_getBlockByNumber',[hex(sealed['finalizedBlock']),False])['hash'] == sealed['finalizedHash']
assert sealed['finalizedBlock'] <= int(f.rpc('eth_getBlockByNumber',['finalized',False])['number'],16)
print(json.dumps({'deployment':'canonically finalized','bindings':bindings,'nextDeployerNonce':sealed['expectedNextDeployerNonce'],'finalizedBlock':sealed['finalizedBlock']}),flush=True)
manifest = ROOT/'deployment.json'
if not manifest.exists():
    args = ['rtk','proxy',str(artifact('beefy-relay')),'tokens-manifest','--source-rpc',launch['aliceRpc'],'--witness-rpc',launch['bobRpc'],'--ethereum-rpc',network['executionWss'],'--wallet',str(ROOT/'hoodi/keys/follower.key'),'--anchor',str(ROOT/'anchor.json'),'--token-stack',str(ROOT/'token-stack/token-stack.json'),'--output',str(manifest)]
    for name,address in bindings.items():
        args.extend(['--'+name,address])
    result = subprocess.run(args,cwd=ROOT,env={**os.environ,**intent['environment']},capture_output=True,text=True)
    with open(ROOT/'hoodi/deployment-manifest.log','a') as log:
        log.write(result.stdout+'\n'+result.stderr+'\nexitCode='+str(result.returncode)+'\n')
        log.flush()
        os.fsync(log.fileno())
    assert result.returncode == 0, 'Manifest verification failed; see private deployment-manifest.log'
print('Authenticated deployment manifest pinned; no transaction was signed or replaced by reconciliation.',flush=True)
