import { PublicClient } from 'viem';

import { createBeaconClient } from './beacon-client.js';
import { createEthereumClient } from './ethereum-client.js';

export async function getSlotByBlockNumber(beaconChainUrl: string, publicClient: PublicClient, blockNumber: bigint) {
  const beaconClient = await createBeaconClient(beaconChainUrl);
  const ethClient = createEthereumClient(publicClient, beaconClient);

  return ethClient.getSlot(blockNumber);
}
