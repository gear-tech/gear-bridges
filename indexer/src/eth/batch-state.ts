import { DataHandlerContext } from '@subsquid/evm-processor';
import { BaseBatchState } from '../common';
import { Store } from '@subsquid/typeorm-store';

export class BatchState extends BaseBatchState<DataHandlerContext<Store, any>> {
  public async save(): Promise<void> {
    await this._processStatuses();
    await this._saveTransfers();
    await this._saveCompletedTransfers();
  }
}
