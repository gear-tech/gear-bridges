#!/usr/bin/env python3
"""Provision a real Safe 1.4.1 for this local-key, test-only Hoodi lane."""
import base64
import fcntl
import hashlib
import json
import os
import sys
import tarfile
import time
from pathlib import Path
from eth_account import Account
from eth_abi import encode, decode
from eth_utils import keccak, to_checksum_address

from run_context import RUN as ROOT, rpc, request, save, wallet, SOURCE, ADDRESS, BEACON
JOURNAL = ROOT / 'safe-setup.json'
ZERO = '0x' + '00' * 20
AMOUNTS = {'deployer': 50*10**15, 'follower': 100*10**15, 'root': 25*10**15, 'paid': 25*10**15, 'campaign': 25*10**15, 'safe-deployer': 15*10**15}
RESERVE = 500*10**15
INTEGRITY = 'fP1jewywSwsIniM04NsqPyVRFKPMAuirC3ftA/TA4X3Zc5EnwQp/UCJUU2PL/37/z/jMo8UUaJ+pnFNWmMU7dQ=='


def calldata(signature, types=(), values=()):
    return '0x' + (keccak(text=signature)[:4] + encode(types, values)).hex()


def call(address, signature, types=(), values=(), returns=('uint256',), block='latest'):
    result = rpc('eth_call', [{'to': address, 'data': calldata(signature, types, values)}, block])
    return decode(returns, bytes.fromhex(result[2:]))


def key(path):
    if not path.exists():
        assert not JOURNAL.exists(), 'Missing sealed key: recover it; never regenerate'
        account = Account.create()
        with open(path, 'x') as f:
            json.dump({'address': account.address, 'private_key': '0x'+account.key.hex()}, f)
            f.flush()
            os.fsync(f.fileno())
        fd = os.open(path.parent, os.O_RDONLY)
        try:
            os.fsync(fd)
        finally:
            os.close(fd)
    return wallet(path)


def submit(record):
    raw = record['signedTransaction']
    assert '0x'+keccak(bytes.fromhex(raw[2:])).hex() == record['txHash']
    assert Account.recover_transaction(raw).lower() == record['from'].lower()
    if rpc('eth_getTransactionReceipt', [record['txHash']]) is None and rpc('eth_getTransactionByHash', [record['txHash']]) is None:
        for tag in ['latest', 'pending']:
            assert int(rpc('eth_getTransactionCount', [record['from'], tag]), 16) <= record['nonce'], 'Unknown nonce consumption: hold original signed transaction'
        assert rpc('eth_sendRawTransaction', [raw]).lower() == record['txHash'].lower()
        print(record['label'], record['txHash'], 'broadcast', flush=True)


def mined(record):
    deadline = time.monotonic() + 1800
    while True:
        receipt = rpc('eth_getTransactionReceipt', [record['txHash']])
        if receipt is not None:
            assert receipt['status'] == '0x1', 'Transaction reverted; preserve journal'
            assert receipt['from'].lower() == record['from'].lower()
            assert (receipt['to'] or '').lower() == (record['to'] or '').lower()
            assert rpc('eth_getBlockByNumber', [receipt['blockNumber'], False])['hash'] == receipt['blockHash'], 'Receipt reorg: hold'
            record['receipt'] = receipt
            save(JOURNAL, state)
            return receipt
        assert time.monotonic() < deadline, 'Pending original transaction: resume journal, never replace'
        time.sleep(30)


def signed(label, signer, nonce, to, data='0x', value=0, gas=None):
    tip = int(rpc('eth_maxPriorityFeePerGas', []), 16)
    maximum = 2*int(rpc('eth_getBlockByNumber', ['latest', False])['baseFeePerGas'], 16) + tip
    request_ = {'from': signer['address'], 'data': data, 'value': hex(value)}
    if to:
        request_['to'] = to
    gas = gas if gas is not None else int(rpc('eth_estimateGas', [request_]), 16)*12//10
    transaction = {'chainId': 560048, 'type': 2, 'nonce': nonce, 'gas': gas, 'maxFeePerGas': maximum, 'maxPriorityFeePerGas': tip, 'data': data, 'value': value}
    if to:
        transaction['to'] = to
    s = Account.sign_transaction(transaction, signer['private_key'])
    return {'label': label, 'from': signer['address'], 'to': to, 'data': data, 'valueWei': str(value), 'nonce': nonce, 'maxCostWei': str(gas*maximum+value), 'txHash': '0x'+s.hash.hex(), 'signedTransaction': '0x'+s.raw_transaction.hex()}


def transact(label, signer, to, data):
    records = [r for r in state['transactions'] if r['label'] == label]
    assert len(records) <= 1
    if records:
        record = records[0]
        assert (record['from'], record['to'], record['data'], record['valueWei']) == (signer['address'], to, data, '0')
    else:
        nonce = int(rpc('eth_getTransactionCount', [signer['address'], 'latest']), 16)
        assert nonce == int(rpc('eth_getTransactionCount', [signer['address'], 'pending']), 16)
        record = signed(label, signer, nonce, to, data)
        assert int(rpc('eth_getBalance', [signer['address'], 'latest']), 16) >= int(record['maxCostWei'])
        state['transactions'].append(record)
        save(JOURNAL, state)
    submit(record)
    return mined(record)


os.umask(0o077)
lock = open(ROOT / 'safe-setup.lock', 'a')
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
assert not JOURNAL.with_suffix('.tmp').exists(), 'Unresolved journal write: reconcile before continuing'
archive = ROOT / 'multisig-artifacts/safe-global-safe-contracts-1.4.1.tgz'
assert base64.b64encode(hashlib.sha512(archive.read_bytes()).digest()).decode() == INTEGRITY
with tarfile.open(archive) as package:
    artifacts = {name: json.load(package.extractfile('package/build/artifacts/contracts/'+folder+'/'+name+'.json')) for name, folder in [('Safe', 'Safe.sol'), ('SafeProxyFactory', 'proxies/SafeProxyFactory.sol'), ('SafeProxy', 'proxies/SafeProxy.sol')]}
assert all(not a['linkReferences'] and not a['deployedLinkReferences'] for a in artifacts.values())
roles = {name: key(ROOT / 'hoodi/keys' / (name+'.key')) for name in AMOUNTS}
owners = [key(ROOT / 'keys' / ('safe-owner-'+str(i)+'.key')) for i in range(1, 6)]
addresses = {name: w['address'] for name, w in roles.items()}
owner_addresses = [w['address'] for w in owners]
assert len(set(a.lower() for a in [*addresses.values(), *owner_addresses, ADDRESS])) == 12
if JOURNAL.exists():
    state = json.loads(JOURNAL.read_text())
    assert state['roles'] == addresses and state['owners'] == owner_addresses and state['testOnly'] is True
else:
    state = {'schemaVersion': 1, 'runId': ROOT.name, 'testOnly': True, 'independentlyControlledRecoveryAuthority': False, 'productionQualification': 'NOT ESTABLISHED', 'publicMigration': 'BLOCKED', 'chainId': 560048, 'safeVersion': '1.4.1', 'packageIntegrity': 'sha512-'+INTEGRITY, 'threshold': 3, 'owners': owner_addresses, 'roles': addresses, 'saltNonce': str(int(ROOT.name.replace('-', ''), 16)), 'transactions': []}
    save(JOURNAL, state)
save(ROOT / 'hoodi/addresses.json', {'roles': addresses, 'testOnly': True})
print('Test-only identities sealed:', json.dumps(addresses), flush=True)
if sys.argv[1:] == ['prepare']:
    raise SystemExit(0)
assert sys.argv[1:] == [], 'Only optional prepare mode is accepted'
assert int(rpc('eth_chainId', []), 16) == 560048
assert rpc('eth_getBlockByNumber', ['0x0', False])['hash'] == '0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b'
header = request(BEACON+'/eth/v1/beacon/headers/finalized')['data']
beacon = request(BEACON+'/eth/v2/beacon/blocks/'+header['root'])['data']['message']
payload = beacon['body']['execution_payload']
assert rpc('eth_getBlockByNumber', [hex(int(payload['block_number'])), False])['hash'] == payload['block_hash']
funding = [r for r in state['transactions'] if r['label'].startswith('fund-')]
if not funding:
    funder = wallet(SOURCE)
    assert funder['address'].lower() == ADDRESS.lower()
    nonce = int(rpc('eth_getTransactionCount', [ADDRESS, 'latest']), 16)
    assert all(int(rpc('eth_getTransactionCount', [ADDRESS, tag]), 16) == nonce for tag in ['pending', 'finalized'])
    funding = [signed('fund-'+name, funder, nonce+i, addresses[name], value=value, gas=21000) for i, (name, value) in enumerate(AMOUNTS.items())]
    balance = int(rpc('eth_getBalance', [ADDRESS, 'latest']), 16)
    assert balance >= RESERVE+sum(int(r['maxCostWei']) for r in funding), 'Insufficient funding after retaining 0.5 ETH reserve'
    state['funding'] = {'source': ADDRESS, 'startingBalanceWei': str(balance), 'retainedReserveWei': str(RESERVE), 'beaconRoot': header['root'], 'beaconSlot': int(beacon['slot'])}
    state['transactions'].extend(funding)
    save(JOURNAL, state)
assert [(r['label'], r['to'], int(r['valueWei'])) for r in funding] == [('fund-'+name, addresses[name], value) for name, value in AMOUNTS.items()]
for record in funding:
    submit(record)
for record in funding:
    mined(record)
signer = roles['safe-deployer']
contracts = {}
for name in ['Safe', 'SafeProxyFactory']:
    receipt = transact('deploy-'+name, signer, None, artifacts[name]['bytecode'])
    address = to_checksum_address(receipt['contractAddress'])
    assert rpc('eth_getCode', [address, 'latest']).lower() == artifacts[name]['deployedBytecode'].lower()
    contracts[name] = address
initializer = calldata('setup(address[],uint256,address,bytes,address,address,uint256,address)', ['address[]','uint256','address','bytes','address','address','uint256','address'], [owner_addresses,3,ZERO,b'',ZERO,ZERO,0,ZERO])
proxy_data = calldata('createProxyWithNonce(address,bytes,uint256)', ['address','bytes','uint256'], [contracts['Safe'],bytes.fromhex(initializer[2:]),int(state['saltNonce'])])
receipt = transact('deploy-safe-proxy', signer, contracts['SafeProxyFactory'], proxy_data)
events = [l for l in receipt['logs'] if l['address'].lower() == contracts['SafeProxyFactory'].lower() and l['topics'][0] == '0x'+keccak(text='ProxyCreation(address,address)').hex()]
assert len(events) == 1 and decode(['address'], bytes.fromhex(events[0]['data'][2:]))[0].lower() == contracts['Safe'].lower()
safe = to_checksum_address('0x'+events[0]['topics'][1][-40:])
assert rpc('eth_getCode', [safe, 'latest']).lower() == artifacts['SafeProxy']['deployedBytecode'].lower()
assert call(safe,'masterCopy()',returns=['address'])[0].lower() == contracts['Safe'].lower()
assert call(safe,'VERSION()',returns=['string'])[0] == '1.4.1'
assert call(safe,'getThreshold()')[0] == 3
assert {a.lower() for a in call(safe,'getOwners()',returns=['address[]'])[0]} == {a.lower() for a in owner_addresses}
assert call(safe,'getModulesPaginated(address,uint256)',['address','uint256'],['0x0000000000000000000000000000000000000001',10],['address[]','address']) == ((), '0x0000000000000000000000000000000000000001')
for slot in ['guard_manager.guard.address','fallback_manager.handler.address']:
    assert int(rpc('eth_getStorageAt',[safe,'0x'+keccak(text=slot).hex(),'latest']),16) == 0
state['contracts'] = contracts | {'wallet': safe}
proof_data = bytes.fromhex(calldata('changeThreshold(uint256)',['uint256'],[3])[2:])
fields = ['address','uint256','bytes','uint8','uint256','uint256','uint256','address','address']
values = [safe,0,proof_data,0,0,0,0,ZERO,ZERO]
hash_ = call(safe,'getTransactionHash(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,uint256)',fields+['uint256'],values+[0],['bytes32'])[0]
signatures = b''.join(bytes(Account.unsafe_sign_hash(hash_,private_key=w['private_key']).signature) for w in sorted(owners,key=lambda w:int(w['address'],16))[:3])
exec_signature = 'execTransaction(address,uint256,bytes,uint8,uint256,uint256,uint256,address,address,bytes)'
try:
    call(safe,exec_signature,fields+['bytes'],values+[signatures[:130]],['bool'])
    raise AssertionError('Two signatures unexpectedly accepted')
except RuntimeError as error:
    assert 'GS020' in str(error), str(error)
state['twoSignatureRejection'] = 'GS020'
proof = transact('three-signature-proof',signer,safe,calldata(exec_signature,fields+['bytes'],values+[signatures]))
successes = [l for l in proof['logs'] if l['address'].lower() == safe.lower() and l['topics'][0] == '0x'+keccak(text='ExecutionSuccess(bytes32,uint256)').hex()]
assert len(successes) == 1 and successes[0]['topics'][1] == '0x'+hash_.hex()
assert decode(['uint256'],bytes.fromhex(successes[0]['data'][2:]))[0] == 0
assert call(safe,'nonce()')[0] == 1 and call(safe,'getThreshold()')[0] == 3
save(JOURNAL,state)
print('Real three-signature transaction mined; waiting for canonical finality.',flush=True)
deadline = time.monotonic()+1800
while True:
    finalized = rpc('eth_getBlockByNumber',['finalized',False])
    receipts = [mined(r) for r in state['transactions']]
    marker = ROOT/'hoodi/funding-complete.json'
    if not marker.exists() and all(int(r['receipt']['blockNumber'],16) <= int(finalized['number'],16) for r in funding):
        save(marker,{'phase':'finalized','chainId':560048,'source':ADDRESS,'finalizedBlock':int(finalized['number'],16),'finalizedHash':finalized['hash'],'roles':addresses,'transactionHashes':[r['txHash'] for r in funding],'testOnly':True})
    if all(int(r['blockNumber'],16) <= int(finalized['number'],16) for r in receipts):
        break
    assert time.monotonic() < deadline, 'Finality deadline: preserve signed journal'
    time.sleep(60)
assert call(safe,'nonce()',block=finalized['number'])[0] == 1
assert call(safe,'getThreshold()',block=finalized['number'])[0] == 3
assert {a.lower() for a in call(safe,'getOwners()',returns=['address[]'],block=finalized['number'])[0]} == {a.lower() for a in owner_addresses}
assert int(rpc('eth_getBalance',[ADDRESS,'latest']),16) >= RESERVE
state['phase'] = 'finalized'
state['finalizedBlock'] = int(finalized['number'],16)
state['finalizedHash'] = finalized['hash']
save(JOURNAL,state)
assert marker.exists(), 'Finalized funding marker missing'
save(ROOT/'recovery-wallet.json',{k:state[k] for k in ['runId','testOnly','independentlyControlledRecoveryAuthority','productionQualification','publicMigration','chainId','safeVersion','packageIntegrity','threshold','owners','contracts','twoSignatureRejection','finalizedBlock','finalizedHash']} | {'threeSignatureProofTransaction':next(r['txHash'] for r in state['transactions'] if r['label']=='three-signature-proof')})
print(json.dumps({'phase':'finalized','testOnly':True,'wallet':safe,'threshold':3,'ownerCount':5,'threeSignatureTransaction':proof['transactionHash'],'twoSignatures':'rejected GS020','finalizedBlock':state['finalizedBlock']}),flush=True)
