import { createPublicClient, http } from 'viem';
import { TypeRegistry } from '@polkadot/types';
import { Keyring } from '@polkadot/api';
import { GearApi } from '@gear-js/api';
import { hoodi } from 'viem/chains';

import { composeProof, createBeaconClient, createEthereumClient } from './index.js';
import { HistoricalProxyTypes } from './ethEvents.js';
import { CheckpointClient, HistoricalProxyClient } from './vara/index.js';

const CHECKPOINT_CLIENT_ID = '0xdb7bbcaff8caa131a94d73f63c8f0dd1fec60e0d263e551d138a9dfb500134ca';
const HISTORICAL_PROXY_ID = '0x5d2a0dcfc30301ad5eda002481e6d0b283f81a1221bef8ba2a3fa65fd56c8e0f';
const CLIENT_ID = '0xd535de98d91b7902a69ccf1a4bf09c061aa76b9012dfc10f744f3212913bdd88';
const CLIENT_ROUTE = '0x3050696e675265636569766572345375626d697452656365697074';
const TX_HASH = '0xddbcd9191bfa11e040afe87d476381066f2aefa7287f846da4eb7c35f5a0a704';
const BEACON_RPC = 'http://unstable.hoodi.beacon-api.nimbus.team/';
const VARA_WS_RPC = 'wss://testnet.vara.network';

const main = async () => {
  const registry = new TypeRegistry();
  registry.setKnownTypes({ types: HistoricalProxyTypes });
  registry.register(HistoricalProxyTypes);

  const publicClient = createPublicClient({ chain: hoodi, transport: http() });

  const beaconClient = await createBeaconClient(BEACON_RPC);

  const ethClient = await createEthereumClient(publicClient, beaconClient);

  const result = await composeProof(beaconClient, ethClient, TX_HASH);
  const slot = result.proof_block.block.slot;

  const encodedEthToVaraEvent = registry.createType('EthToVaraEvent', result).toHex();

  console.log(encodedEthToVaraEvent);

  const gearApi = await GearApi.create({ providerAddress: VARA_WS_RPC });

  const checkpoint = new CheckpointClient(gearApi, CHECKPOINT_CLIENT_ID);

  let slotTracked = false;

  while (!slotTracked) {
    const _result = await checkpoint.serviceCheckpointFor.get(slot);

    if ('ok' in _result) {
      slotTracked = true;
    } else {
      if (_result.err === 'NotPresent') {
        console.log(`Checkpoint: slot ${slot} not present yet. Waiting...`);
        await new Promise((resolve) => setTimeout(resolve, 3000));
      } else {
        throw new Error(`Checkpoint error: slot ${slot} outdated`);
      }
    }
  }

  const historicalProxy = new HistoricalProxyClient(gearApi, HISTORICAL_PROXY_ID);

  const keyring = new Keyring({ type: 'sr25519', ss58Format: 137 });
  const account = keyring.createFromUri('//Alice');

  const tx = await historicalProxy.historicalProxy
    .redirect(slot, encodedEthToVaraEvent, CLIENT_ID, CLIENT_ROUTE)
    .withAccount(account)
    .calculateGas();

  const { blockHash, txHash, response } = await tx.signAndSend();

  console.log(`Transaction sent: ${txHash}, block: ${blockHash}`);

  const hpResult = await response();

  console.log(hpResult);
};

main();
