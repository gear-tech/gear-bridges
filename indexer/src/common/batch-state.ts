import { FindManyOptions, Store } from '@subsquid/typeorm-store';
import { CompletedTransfer, Network, Pair, Status, Transfer } from '../model';
import { DataHandlerContext as SubstrateContext } from '@subsquid/substrate-processor';
import { DataHandlerContext as EthereumContext } from '@subsquid/evm-processor';
import { In, IsNull, LessThanOrEqual, MoreThanOrEqual } from 'typeorm';
import { randomUUID } from 'node:crypto';

export class BaseBatchState<Context extends SubstrateContext<Store, any> | EthereumContext<Store, any>> {
  protected _transfers: Map<string, Transfer>;
  protected _completed: Map<string, CompletedTransfer>;
  protected _pairs: Map<string, Pair>;
  protected _statuses: Map<string, Status>;
  protected _ctx: Context;

  constructor(private _network: Network) {
    this._transfers = new Map();
    this._completed = new Map();
    this._statuses = new Map();
    this._pairs = new Map();
  }

  public async new(ctx: Context) {
    this._ctx = ctx;
    this._transfers.clear();
    this._completed.clear();
    this._pairs.clear();
    this._statuses.clear();

    await this._loadPairs();
    await this._loadCompleted();
  }

  protected async _loadPairs(activeOnBlock: 'latest' | bigint = 'latest') {
    const condition: FindManyOptions<Pair> =
      activeOnBlock === 'latest'
        ? { where: { isRemoved: false, upgradedTo: IsNull() } }
        : {
            where: [
              { activeSinceBlock: LessThanOrEqual(activeOnBlock), activeToBlock: IsNull() },
              { activeSinceBlock: LessThanOrEqual(activeOnBlock), activeToBlock: MoreThanOrEqual(activeOnBlock) },
            ],
          };

    const pairs = await this._ctx.store.find(Pair, condition);

    for (const pair of pairs) {
      if (this._network === Network.Ethereum) {
        this._pairs.set(pair.ethToken, pair);
      } else {
        this._pairs.set(pair.varaToken, pair);
      }
    }
  }

  private async _loadCompleted() {
    const completed = await this._ctx.store.find(CompletedTransfer, { where: { destNetwork: this._network } });

    for (const transfer of completed) {
      this._completed.set(transfer.nonce, transfer);
    }

    if (completed.length > 0) {
      this._ctx.log.debug(`Loaded ${completed.length} completed transfers`);
    }
  }

  protected async _getDestinationAddress(sourceId: string, blockNumber: bigint): Promise<string> {
    const src = sourceId.toLowerCase();
    const retryLimit = 30;
    let retryCount = 0;

    let pair = this._pairs.get(src);
    while (!pair) {
      this._ctx.log.warn(`Pair not found for ${src}, retrying...`);
      await new Promise((resolve) => setTimeout(resolve, 1000));
      await this._loadPairs(blockNumber);
      pair = this._pairs.get(src);
      retryCount++;
      if (retryCount >= retryLimit) {
        this._ctx.log.error(`Failed to load pair for ${src}. Exiting`);
        process.exit(1);
      }
    }

    if (this._network === Network.Ethereum) {
      return pair.varaToken.toLowerCase();
    } else {
      return pair.ethToken.toLowerCase();
    }
  }

  private _queryTransfers(nonces: string[]): Promise<Transfer[]> {
    return this._ctx.store.find(Transfer, { where: { nonce: In(nonces) } });
  }

  protected async _saveCompletedTransfers(): Promise<void> {
    if (this._completed.size === 0) return;

    const transfers = await this._queryTransfers(Array.from(this._completed.keys()));
    const completedToDelete: CompletedTransfer[] = [];
    const transfersToUpdate: Transfer[] = [];

    if (transfers.length > 0) {
      for (const transfer of transfers) {
        const completed = this._completed.get(transfer.nonce)!;
        transfer.status = Status.Completed;
        transfer.completedAt = completed.timestamp;
        transfersToUpdate.push(transfer);
        completedToDelete.push(completed);
        this._completed.delete(transfer.nonce);
      }

      if (transfersToUpdate.length > 0) {
        await this._ctx.store.save(transfersToUpdate);
      }
      if (completedToDelete.length > 0) {
        await this._ctx.store.remove(completedToDelete);
      }
    }

    if (this._completed.size > 0) {
      const savedCompletedTransfers = await this._ctx.store.findBy(CompletedTransfer, {
        nonce: In(Array.from(this._completed.keys())),
      });
      const completedToSave = Array.from(this._completed.values()).filter(
        ({ nonce }) => !savedCompletedTransfers.some((saved) => saved.nonce === nonce),
      );
      await this._ctx.store.save(completedToSave);
      this._ctx.log.debug(`Saved ${this._completed.size} completed transfers`);
    }
  }

  protected async _processStatuses() {
    if (this._statuses.size === 0) return;

    const noncesToUpdate = Array.from(this._statuses.keys());
    const noncesNotInCache = noncesToUpdate.filter((nonce) => !this._transfers.has(nonce));

    if (noncesNotInCache.length > 0) {
      const transfers = await this._ctx.store.find(Transfer, {
        where: { nonce: In(noncesNotInCache), sourceNetwork: this._network },
      });
      for (const transfer of transfers) {
        this._transfers.set(transfer.nonce, transfer);
      }
    }

    for (const [nonce, status] of this._statuses.entries()) {
      const transfer = this._transfers.get(nonce);
      if (transfer) {
        transfer.status = status;
      } else {
        this._ctx.log.error(`${nonce}: Transfer not found in cache or DB for status update`);
      }
    }
  }

  protected async _saveTransfers() {
    if (this._transfers.size === 0) return;

    await this._ctx.store.save(Array.from(this._transfers.values()));

    this._ctx.log.info(`Saved ${this._transfers.size} transfers`);
  }

  public async save() {
    await this._processStatuses();
    await this._saveTransfers();
    await this._saveCompletedTransfers();
  }

  public async addTransfer(transfer: Transfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    transfer.source = transfer.source.toLowerCase();
    transfer.destination = await this._getDestinationAddress(transfer.source, transfer.blockNumber);
    transfer.sender = transfer.sender.toLowerCase();
    transfer.receiver = transfer.receiver.toLowerCase();
    transfer.nonce = transfer.nonce;
    this._transfers.set(transfer.nonce, transfer);

    this._ctx.log.info(`${transfer.nonce}: Transfer requested in block ${transfer.blockNumber}`);
  }

  public async updateTransferStatus(nonce: string, status: Status) {
    if (this._statuses.get(nonce) === status) return;

    this._statuses.set(nonce, status);
    this._ctx.log.info(`${nonce}: Status changed to ${status}`);
  }

  public setCompletedTransfer(nonce: string, timestamp: Date) {
    this._completed.set(
      nonce,
      new CompletedTransfer({
        id: randomUUID(),
        nonce,
        timestamp,
        destNetwork: this._network,
      }),
    );

    this._ctx.log.info(`${nonce}: Transfer completed`);
  }

  protected async _getTransfer(nonce: string): Promise<Transfer | undefined> {
    if (this._transfers.has(nonce)) {
      return this._transfers.get(nonce);
    }
    const t = await this._ctx.store.findOneBy(Transfer, { nonce });
    if (t) {
      this._transfers.set(nonce, t);
    }
    return t;
  }
}
