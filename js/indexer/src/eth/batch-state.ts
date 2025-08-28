import { DataHandlerContext } from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';

import { BaseBatchState } from '../common';
import { Status } from '../model';

export class BatchState extends BaseBatchState<DataHandlerContext<Store, any>> {
  private _paidRequests: Set<string>;

  protected _clear(): void {
    super._clear();
    this._paidRequests.clear();
  }

  public async save(): Promise<void> {
    await this._processStatuses();
    await this._processPaidRequests();
    await this._saveTransfers();
    await this._saveCompletedTransfers();
    await this._processCompletedTransfers();
  }

  private async _processPaidRequests() {
    for (const [_nonce, transfer] of this._transfers.entries()) {
      if (this._paidRequests.has(transfer.txHash)) {
        transfer.status = Status.Bridging;
        this._paidRequests.delete(transfer.txHash);
      }
    }
  }

  public bridgingPaid(txHash: string) {
    this._paidRequests.add(txHash.toLowerCase());
  }
}
