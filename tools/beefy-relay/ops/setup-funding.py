#!/usr/bin/env python3
"""Prepare and fund five test-only Hoodi roles with original signed-transaction journals."""
import fcntl
import json
import os
import sys
import time
from eth_account import Account
from eth_utils import keccak

from run_context import RUN as ROOT, rpc, request, save, wallet, SOURCE, ADDRESS, BEACON

JOURNAL = ROOT / 'funding-setup.json'
AMOUNTS = {'deployer': 50*10**15, 'follower': 100*10**15, 'root': 25*10**15, 'paid': 25*10**15, 'campaign': 25*10**15}
RESERVE = 500*10**15


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


def mined(record, state):
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


def signed(label, signer, nonce, to, value):
    tip = int(rpc('eth_maxPriorityFeePerGas', []), 16)
    maximum = 2*int(rpc('eth_getBlockByNumber', ['latest', False])['baseFeePerGas'], 16) + tip
    gas = 21000
    transaction = {'chainId': 560048, 'type': 2, 'nonce': nonce, 'gas': gas, 'maxFeePerGas': maximum, 'maxPriorityFeePerGas': tip, 'to': to, 'data': '0x', 'value': value}
    original = Account.sign_transaction(transaction, signer['private_key'])
    return {'label': label, 'from': signer['address'], 'to': to, 'data': '0x', 'valueWei': str(value), 'nonce': nonce, 'maxCostWei': str(gas*maximum+value), 'txHash': '0x'+original.hash.hex(), 'signedTransaction': '0x'+original.raw_transaction.hex()}


def main():
    assert sys.argv[1:] in ([], ['prepare']), 'Only optional prepare mode is accepted'
    os.umask(0o077)
    with open(ROOT / 'funding-setup.lock', 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        assert not JOURNAL.with_suffix('.tmp').exists(), 'Unresolved journal write: reconcile before continuing'
        roles = {name: key(ROOT / 'hoodi/keys' / (name+'.key')) for name in AMOUNTS}
        addresses = {name: w['address'] for name, w in roles.items()}
        assert len(set(a.lower() for a in [*addresses.values(), ADDRESS])) == 6
        if JOURNAL.exists():
            state = json.loads(JOURNAL.read_text())
            assert state['runId'] == ROOT.name and state['chainId'] == 560048 and state['roles'] == addresses and state['testOnly'] is True
        else:
            state = {'schemaVersion': 1, 'runId': ROOT.name, 'testOnly': True, 'productionQualification': 'NOT ESTABLISHED', 'publicMigration': 'BLOCKED', 'chainId': 560048, 'roles': addresses, 'transactions': []}
            save(JOURNAL, state)
        save(ROOT / 'hoodi/addresses.json', {'roles': addresses, 'testOnly': True})
        print('Test-only identities sealed:', json.dumps(addresses), flush=True)
        if sys.argv[1:] == ['prepare']:
            return
        assert int(rpc('eth_chainId', []), 16) == 560048
        assert rpc('eth_getBlockByNumber', ['0x0', False])['hash'] == '0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b'
        header = request(BEACON+'/eth/v1/beacon/headers/finalized')['data']
        beacon = request(BEACON+'/eth/v2/beacon/blocks/'+header['root'])['data']['message']
        payload = beacon['body']['execution_payload']
        assert rpc('eth_getBlockByNumber', [hex(int(payload['block_number'])), False])['hash'] == payload['block_hash']
        funding = state['transactions']
        if not funding:
            funder = wallet(SOURCE)
            assert funder['address'].lower() == ADDRESS.lower()
            nonce = int(rpc('eth_getTransactionCount', [ADDRESS, 'latest']), 16)
            assert all(int(rpc('eth_getTransactionCount', [ADDRESS, tag]), 16) == nonce for tag in ['pending', 'finalized'])
            funding = [signed('fund-'+name, funder, nonce+i, addresses[name], value) for i, (name, value) in enumerate(AMOUNTS.items())]
            balance = int(rpc('eth_getBalance', [ADDRESS, 'latest']), 16)
            assert balance >= RESERVE+sum(int(r['maxCostWei']) for r in funding), 'Insufficient funding after retaining 0.5 ETH reserve'
            state['funding'] = {'source': ADDRESS, 'startingBalanceWei': str(balance), 'retainedReserveWei': str(RESERVE), 'beaconRoot': header['root'], 'beaconSlot': int(beacon['slot'])}
            state['transactions'] = funding
            save(JOURNAL, state)
        assert [(r['label'], r['from'].lower(), r['to'], r['data'], int(r['valueWei'])) for r in funding] == [('fund-'+name, ADDRESS.lower(), addresses[name], '0x', value) for name, value in AMOUNTS.items()]
        for record in funding:
            submit(record)
        print('Original funding transactions submitted; waiting for canonical finality.', flush=True)
        deadline = time.monotonic()+1800
        while True:
            finalized = rpc('eth_getBlockByNumber', ['finalized', False])
            receipts = [mined(record, state) for record in funding]
            if all(int(receipt['blockNumber'], 16) <= int(finalized['number'], 16) for receipt in receipts):
                break
            assert time.monotonic() < deadline, 'Finality deadline: preserve signed journal'
            time.sleep(60)
        assert int(rpc('eth_getBalance', [ADDRESS, 'latest']), 16) >= RESERVE
        marker = ROOT/'hoodi/funding-complete.json'
        if marker.exists():
            recorded = json.loads(marker.read_text())
            assert recorded['phase'] == 'finalized' and recorded['chainId'] == 560048 and recorded['source'] == ADDRESS and recorded['roles'] == addresses and recorded['transactionHashes'] == [r['txHash'] for r in funding] and recorded['testOnly'] is True
            assert recorded['finalizedBlock'] <= int(finalized['number'], 16) and rpc('eth_getBlockByNumber', [hex(recorded['finalizedBlock']), False])['hash'] == recorded['finalizedHash'], 'Original funding finality is not canonical'
        else:
            save(marker, {'phase': 'finalized', 'chainId': 560048, 'source': ADDRESS, 'finalizedBlock': int(finalized['number'], 16), 'finalizedHash': finalized['hash'], 'roles': addresses, 'transactionHashes': [r['txHash'] for r in funding], 'testOnly': True})
            recorded = json.loads(marker.read_text())
        state['phase'] = 'finalized'
        state['finalizedBlock'] = recorded['finalizedBlock']
        state['finalizedHash'] = recorded['finalizedHash']
        save(JOURNAL, state)
        print(json.dumps({'phase': 'finalized', 'testOnly': True, 'roles': addresses, 'finalizedBlock': state['finalizedBlock']}), flush=True)


if __name__ == '__main__':
    main()
