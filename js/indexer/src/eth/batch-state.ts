import { DataHandlerContext } from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';

import { MerkleRootInMessageQueue, Network, Status } from '../model/index.js';
import { BaseBatchState, mapKeys, mapValues } from '../common/index.js';
import { In } from 'typeorm';

export class BatchState extends BaseBatchState<DataHandlerContext<Store, any>> {
  private _paidRequests: Set<string>;
  private _merkleRoots: Map<string, MerkleRootInMessageQueue>;

  constructor() {
    super(Network.Ethereum, Network.Vara);
    this._paidRequests = new Set();
    this._merkleRoots = new Map();
  }

  protected _clear(): void {
    super._clear();
    this._paidRequests.clear();
    this._merkleRoots.clear();
  }

  public async save(): Promise<void> {
    await this._processStatuses();
    await this._savePaidRequests();
    await this._saveTransfers();
    await this._saveMerkleRoots();
    await this._saveCompletedTransfers();
    await this._processCompletedTransfers();
  }

  private async _savePaidRequests() {
    for (const [_nonce, transfer] of this._transfers.entries()) {
      if (this._paidRequests.has(transfer.txHash)) {
        transfer.status = Status.Bridging;
        this._paidRequests.delete(transfer.txHash);
      }
    }
  }

  private async _saveMerkleRoots() {
    if (this._merkleRoots.size === 0) return;

    const savedBlocks = await this._ctx.store.findBy(MerkleRootInMessageQueue, {
      blockNumber: In(mapKeys(this._merkleRoots)),
    });

    const values = mapValues(this._merkleRoots);
    const savedBlockNumbers: bigint[] = [];

    if (savedBlocks.length > 0) {
      this._log.warn(
        {
          saved: savedBlocks.map(({ blockNumber, merkleRoot }) => ({ bn: blockNumber, mr: merkleRoot })),
          duplicates: values.map(({ blockNumber, merkleRoot }) => ({
            bn: blockNumber,
            mr: merkleRoot,
          })),
        },
        `Merkle root duplicates found`,
      );

      savedBlockNumbers.push(...savedBlocks.map(({ blockNumber }) => blockNumber));
    }

    await this._ctx.store.save(values.filter(({ blockNumber }) => savedBlockNumbers.includes(blockNumber)));

    this._log.info({ count: this._merkleRoots.size }, 'Merkle roots saved');
  }

  public bridgingPaid(txHash: string) {
    this._log.info({ txHash }, 'Bridging paid');
    this._paidRequests.add(txHash.toLowerCase());
  }

  public newMerkleRoot(blockNumber: bigint, merkleRoot: string) {
    this._log.info({ blockNumber, merkleRoot }, 'Merkle root received');
    this._merkleRoots.set(blockNumber.toString(), new MerkleRootInMessageQueue({ blockNumber, merkleRoot }));
  }
}
