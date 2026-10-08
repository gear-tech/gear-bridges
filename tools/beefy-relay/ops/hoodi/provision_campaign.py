import fcntl
import json
import os
import time
import subprocess
from pathlib import Path
from eth_account import Account
from eth_abi import encode
from eth_utils import keccak, to_checksum_address
import sys
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import run_context as funding
from run_context import RUN as run, rpc, save, wallet

os.umask(0o077)
root = run / 'hoodi'
funding.RPC = json.loads((root / 'network-gate.json').read_text())['executionHttp']
lock = open(root / 'campaign-inventory.lock', 'a')
fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
finalized = json.loads((root / 'deployment-finalized.json').read_text())
assert finalized['phase'] == 'finalized'
assert rpc('eth_getBlockByNumber', [hex(finalized['finalizedBlock']), False])['hash'] == finalized['finalizedHash']
owner_nonce = finalized['expectedNextDeployerNonce']
assert int(rpc('eth_chainId', []), 16) == 560048
assert rpc('eth_getBlockByNumber', ['0x0', False])['hash'] == '0xbbe312868b376a3001692a646dd2d7d1e4406380dfd86b98aa8a34d1557c971b'
broadcast = json.loads((run / 'forge-final/broadcast/BeefyTokens.s.sol/560048/run-latest.json').read_text())
names = ['CircleToken', 'TetherToken', 'WrappedBitcoin', 'WrappedEther']
contracts = {name: to_checksum_address(next(t['contractAddress'] for t in broadcast['transactions'] if t['contractName'] == name)) for name in names}
source_inventory = None
if funding.CONFIG.get("runtimeProfile") is not None:
    launch = funding.load(run / "source-chain" / "launch-state.json")
    funding.require(launch["phase"] == "ready" and launch["identity"]["runtimeProfile"] == funding.CONFIG["runtimeProfile"],
                    "Normal source launch is not ready or its approved profile changed")
    funding.require(launch["aliceRpc"] == funding.CONFIG["source"]["aliceRpc"]
                    and launch["bobRpc"] == funding.CONFIG["source"]["bobRpc"], "Source inventory endpoint mismatch")
    subprocess.run([str(funding.artifact("beefy-relay")), "tokens-provision-source-inventory",
        "--source-rpc", launch["aliceRpc"], "--witness-rpc", launch["bobRpc"],
        "--expected-genesis", launch["readiness"]["genesisHash"],
        "--gear-suri-file", str(root / "gear-keys" / "governance.suri"),
        "--campaign-suri-file", str(root / "gear-keys" / "campaign.suri"),
        "--deployment-manifest", str(run / "deployment.json"),
        "--token-stack", str(run / "token-stack" / "token-stack.json")], check=True)
    source_inventory = funding.load(run / "token-stack" / "token-stack.json")["sourceInventory"]
    funding.require(source_inventory["status"] == "ready", "Source inventory lacks finalized evidence")
owner, campaign = (wallet(root / 'keys' / (role + '.key')) for role in ['deployer', 'campaign'])
journal = root / 'campaign-inventory.json'
assert not journal.with_suffix('.tmp').exists(), 'Unresolved inventory journal write'
balance_data = '0x' + (keccak(text='balanceOf(address)')[:4] + encode(['address'], [campaign['address']])).hex()
if journal.exists():
    state = json.loads(journal.read_text())
    assert state['contracts'] == contracts and state['campaign'] == campaign['address'] and state['amount'] == 100
else:
    assert all(int(rpc('eth_call', [{'to': address, 'data': balance_data}, 'latest']), 16) == 0 for address in contracts.values()), 'Existing inventory: reconcile, do not mint twice'
    for signer, nonce in [(owner, owner_nonce), (campaign, 0)]:
        assert int(rpc('eth_getTransactionCount', [signer['address'], 'latest']), 16) == int(rpc('eth_getTransactionCount', [signer['address'], 'pending']), 16) == nonce
    tip = int(rpc('eth_maxPriorityFeePerGas', []), 16)
    maximum = 2 * int(rpc('eth_getBlockByNumber', ['latest', False])['baseFeePerGas'], 16) + tip
    state = {'chainId': 560048, 'contracts': contracts, 'campaign': campaign['address'], 'amount': 100, 'transactions': []}
    for i, name in enumerate(names):
        deposit = name == 'WrappedEther'
        signer = campaign if deposit else owner
        if not deposit:
            assert rpc('eth_call', [{'to': contracts[name], 'data': '0x8da5cb5b'}, 'latest'])[-40:].lower() == owner['address'][2:].lower()
        data = '0x' + (keccak(text='deposit()')[:4] if deposit else keccak(text='mint(address,uint256)')[:4] + encode(['address', 'uint256'], [campaign['address'], 100])).hex()
        gas = int(rpc('eth_estimateGas', [{'from': signer['address'], 'to': contracts[name], 'data': data, 'value': hex(100 if deposit else 0)}]), 16) * 12 // 10
        tx = {'chainId': 560048, 'type': 2, 'to': contracts[name], 'data': data, 'value': 100 if deposit else 0, 'nonce': 0 if deposit else owner_nonce+i, 'gas': gas, 'maxFeePerGas': maximum, 'maxPriorityFeePerGas': tip}
        signed = Account.sign_transaction(tx, signer['private_key'])
        state['transactions'].append({'asset': name, 'from': signer['address'], 'to': contracts[name], 'nonce': tx['nonce'], 'maxCostWei': gas*maximum+tx['value'], 'txHash': '0x'+signed.hash.hex(), 'signedTransaction': '0x'+signed.raw_transaction.hex()})
    for signer in [owner, campaign]:
        assert int(rpc('eth_getBalance', [signer['address'], 'latest']), 16) > sum(t['maxCostWei'] for t in state['transactions'] if t['from'] == signer['address'])
    save(journal, state)
for tx in state['transactions']:
    if rpc('eth_getTransactionReceipt', [tx['txHash']]) is None and rpc('eth_getTransactionByHash', [tx['txHash']]) is None:
        assert int(rpc('eth_getTransactionCount', [tx['from'], 'latest']), 16) <= tx['nonce']
        assert int(rpc('eth_getTransactionCount', [tx['from'], 'pending']), 16) <= tx['nonce'], 'Unknown nonce consumption: hold original transaction'
        assert rpc('eth_sendRawTransaction', [tx['signedTransaction']]).lower() == tx['txHash'].lower()
        tx['broadcast'] = True
        save(journal, state)
        print(tx['asset'], tx['txHash'], 'broadcast', flush=True)
deadline = time.monotonic()+1800
while True:
    final = rpc('eth_getBlockByNumber', ['finalized', False])
    complete = True
    for tx in state['transactions']:
        receipt = rpc('eth_getTransactionReceipt', [tx['txHash']])
        if receipt is None:
            complete = False
            continue
        assert receipt['status'] == '0x1' and receipt['to'].lower() == tx['to'].lower()
        assert rpc('eth_getBlockByNumber', [receipt['blockNumber'], False])['hash'] == receipt['blockHash']
        tx['receipt'] = receipt
        complete &= int(receipt['blockNumber'], 16) <= int(final['number'], 16)
    save(journal, state)
    if complete:
        balances = {name: int(rpc('eth_call', [{'to': address, 'data': balance_data}, final['number']]), 16) for name, address in contracts.items()}
        assert set(balances.values()) == {100}
        result = {"phase": "finalized", "balancesRaw": balances, "finalizedBlock": int(final["number"], 16), "finalizedHash": final["hash"], "testOnly": True, "transactionHashes": [t["txHash"] for t in state["transactions"]]}
        if source_inventory is not None:
            result["sourceInventory"] = source_inventory
        save(root / "campaign-inventory-finalized.json", result)
        print('Campaign owns exactly 100 raw units of each fixture asset at a finalized block.', flush=True)
        break
    assert time.monotonic() < deadline, 'Inventory not finalized: preserve original signed transactions'
    time.sleep(60)
