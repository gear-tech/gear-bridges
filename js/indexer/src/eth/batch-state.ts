import { DataHandlerContext } from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';

import { MerkleRootInMessageQueue, Network, Status } from '../model/index.js';
import { BaseBatchState, setValues } from '../common/index.js';

export class BatchState extends BaseBatchState<DataHandlerContext<Store, any>> {
  private _paidRequests: Set<string>;
  private _merkleRoots: Set<MerkleRootInMessageQueue>;

  constructor() {
    super(Network.Ethereum, Network.Vara);
    this._paidRequests = new Set();
    this._merkleRoots = new Set();
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

    await this._ctx.store.save(setValues(this._merkleRoots));

    this._log.info(`${this._merkleRoots.size} saved`);
  }

  public bridgingPaid(txHash: string) {
    this._log.info({ txHash }, `Bridging paid`);
    this._paidRequests.add(txHash.toLowerCase());
  }

  public newMerkleRoot(blockNumber: bigint, merkleRoot: string) {
    this._log.info(`Received merkle root for block ${blockNumber}`);
    this._merkleRoots.add(new MerkleRootInMessageQueue({ blockNumber, merkleRoot }));
  }
}
