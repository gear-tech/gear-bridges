import { GetBlockReturnType, PublicClient, TransactionReceipt } from 'viem';
import { BeaconClient } from './beacon-client';

const BLOCK_TIME = 12;

export interface EthereumClient {
  getSlot(blockNumber: bigint | number): Promise<number>;
  getTransactionReceipt(hash: `0x${string}`): Promise<TransactionReceipt>;
  getBlockByHash(hash: `0x${string}`): Promise<GetBlockReturnType>;
}

class _EthereumClient implements EthereumClient {
  constructor(
    private rpc: PublicClient,
    private beaconGenesisTime: number,
  ) {}

  public async getSlot(blockNumber: bigint | number) {
    const block = await this.rpc.getBlock({ blockNumber: BigInt(blockNumber) });

    const slot = (Number(block.timestamp) - this.beaconGenesisTime) / BLOCK_TIME;

    return slot;
  }

  public getTransactionReceipt(hash: `0x${string}`) {
    return this.rpc.getTransactionReceipt({ hash });
  }

  public getBlockByHash(hash: `0x${string}`) {
    return this.rpc.getBlock({ blockHash: hash });
  }

  public getReceipts() {}
}

export function createEthereumClient(client: PublicClient, beaconClient: BeaconClient): EthereumClient {
  return new _EthereumClient(client, beaconClient.genesisBlockTime);
}
