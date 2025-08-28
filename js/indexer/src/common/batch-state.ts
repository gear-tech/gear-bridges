import { DataHandlerContext as SubstrateContext } from '@subsquid/substrate-processor';
import { DataHandlerContext as EthereumContext } from '@subsquid/evm-processor';
import { In, IsNull, LessThanOrEqual, MoreThanOrEqual, Not } from 'typeorm';
import { FindManyOptions, Store } from '@subsquid/typeorm-store';
import { Logger } from '@subsquid/logger';

import { CompletedTransfer, Network, Pair, Status, Transfer } from '../model/index.js';
import { mapKeys, mapValues } from './map.js';

const PAIR_RETRY_LIMIT = 5;
const PAIR_RETRY_DELAY = 1000;

export abstract class BaseBatchState<Context extends SubstrateContext<Store, any> | EthereumContext<Store, any>> {
  protected _transfers: Map<string, Transfer>;
  protected _completed: Map<string, CompletedTransfer>;
  protected _pairs: Map<string, Pair>;
  protected _statuses: Map<string, Status>;
  protected _ctx: Context;
  protected _log: Logger;

  constructor(
    private _network: Network,
    private _counterpartNetwork: Network,
  ) {
    this._transfers = new Map();
    this._completed = new Map();
    this._statuses = new Map();
    this._pairs = new Map();
  }

  protected _clear() {
    this._transfers.clear();
    this._completed.clear();
    this._pairs.clear();
    this._statuses.clear();
  }

  public async new(ctx: Context) {
    this._ctx = ctx;
    this._log = ctx.log.child(this._network);

    this._clear();

    await this._loadPairs('latest');
  }

  protected async _loadPairs(activeOnBlock?: 'latest' | bigint) {
    let condition: FindManyOptions<Pair>;
    if (activeOnBlock) {
      condition =
        activeOnBlock === 'latest'
          ? { where: { isRemoved: false, upgradedTo: IsNull() } }
          : {
              where: [
                { activeSinceBlock: LessThanOrEqual(activeOnBlock), activeToBlock: IsNull() },
                { activeSinceBlock: LessThanOrEqual(activeOnBlock), activeToBlock: MoreThanOrEqual(activeOnBlock) },
              ],
            };
    } else {
      condition = {};
    }

    const pairs = await this._ctx.store.find(Pair, condition);

    for (const pair of pairs) {
      if (this._network === Network.Ethereum) {
        this._pairs.set(pair.ethToken, pair);
      } else {
        this._pairs.set(pair.varaToken, pair);
      }
    }
  }

  protected async _getDestinationAddress(sourceId: string): Promise<string> {
    const src = sourceId.toLowerCase();
    let retryCount = 0;

    let pair = this._pairs.get(src);
    while (!pair) {
      this._log.warn(`Pair not found for ${src}, retrying...`);
      await new Promise((resolve) => setTimeout(resolve, PAIR_RETRY_DELAY));

      if (this._network === Network.Ethereum) {
        pair = await this._ctx.store.get(Pair, { where: { ethToken: src } });
        if (pair) {
          this._pairs.set(pair.ethToken, pair);
        }
      } else {
        pair = await this._ctx.store.get(Pair, { where: { varaToken: src } });
        if (pair) {
          this._pairs.set(pair.varaToken, pair);
        }
      }
      retryCount++;
      if (retryCount >= PAIR_RETRY_LIMIT) {
        this._log.error(`Failed to load pair for ${src}. Exiting`);
        throw new Error('Pair not found');
      }
    }

    if (this._network === Network.Ethereum) {
      return pair.varaToken.toLowerCase();
    } else {
      return pair.ethToken.toLowerCase();
    }
  }

  protected async _processCompletedTransfers(): Promise<void> {
    const completed = await this._ctx.store.find(CompletedTransfer, { where: { srcNetwork: this._network } });

    if (completed.length === 0) return;

    const nonces = completed.map((info) => info.id);

    const transfers = await this._ctx.store.find(Transfer, {
      where: { nonce: In(nonces), sourceNetwork: this._network },
    });

    if (transfers.length === 0) return;

    const completedToRemove: CompletedTransfer[] = [];

    for (const transfer of transfers) {
      const completedInfo = completed.find((info) => info.id === transfer.nonce)!;
      transfer.status = Status.Completed;
      transfer.completedAt = completedInfo.timestamp;
      transfer.completedAtBlock = completedInfo.blockNumber;
      transfer.completedAtTxHash = completedInfo.txHash;
      completedToRemove.push(completedInfo);
    }

    await this._ctx.store.save(transfers);
    this._log.info(`${transfers.length} transfers marked as completed`);
    this._log.debug({ nonces: transfers.map((transfer) => transfer.nonce) });

    await this._ctx.store.remove(completedToRemove);
    this._log.info(`${completedToRemove.length} completed records removed`);
    this._log.debug({ nonces: completedToRemove.map((transfer) => transfer.id) });
  }

  protected async _saveCompletedTransfers(): Promise<void> {
    if (this._completed.size === 0) return;

    const duplicates = await this._ctx.store.find(CompletedTransfer, {
      where: { id: In(mapKeys(this._completed)) },
    });

    const nonces = duplicates.map((info) => info.id);

    if (duplicates.length > 0) {
      this._log.info(`Found ${duplicates.length} duplicates of completed transfers`);

      for (const duplicate of duplicates) {
        this._log.info(
          {
            nonce: duplicate.id,
            blockNumber: duplicate.blockNumber,
            txHash: duplicate.txHash,
            pendingBlockNumber: this._completed.get(duplicate.id)!.blockNumber,
            pendingTxHash: this._completed.get(duplicate.id)!.txHash,
          },
          'Duplicate completed transfer found',
        );
      }
    }

    const completedToSave = mapValues(this._completed).filter(({ id }) => !nonces.includes(id));

    if (completedToSave.length === 0) return;

    await this._ctx.store.save(completedToSave);
    this._log.info(`Saved ${completedToSave.length} completed records`);
    this._log.debug({ nonces: completedToSave.map((info) => info.id) });
  }

  protected async _processStatuses() {
    if (this._statuses.size === 0) return;

    const noncesToUpdate = mapKeys(this._statuses);
    const noncesNotInCache = noncesToUpdate.filter((nonce) => !this._transfers.has(nonce));

    if (noncesNotInCache.length > 0) {
      const transfers = await this._ctx.store.find(Transfer, {
        where: { nonce: In(noncesNotInCache), sourceNetwork: this._network, status: Not(Status.Completed) },
      });
      for (const transfer of transfers) {
        this._transfers.set(transfer.nonce, transfer);
      }
    }

    for (const [nonce, status] of this._statuses.entries()) {
      const transfer = this._transfers.get(nonce);
      if (transfer) {
        if (transfer.status !== Status.Completed) {
          transfer.status = status;
        }
      } else {
        this._log.error(`${nonce}: Transfer not found in cache or DB for status update`);
      }
    }
  }

  protected async _saveTransfers() {
    if (this._transfers.size === 0) return;

    await this._ctx.store.save(mapValues(this._transfers));

    this._log.info(`Saved ${this._transfers.size} transfers`);
  }

  public abstract save(): Promise<void>;

  public async addTransfer(transfer: Transfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    transfer.source = transfer.source.toLowerCase();
    transfer.sender = transfer.sender.toLowerCase();
    transfer.receiver = transfer.receiver.toLowerCase();
    try {
      transfer.destination = await this._getDestinationAddress(transfer.source);
    } catch (error) {
      this._log.error(
        { block: transfer.blockNumber, tx: transfer.txHash, nonce: transfer.nonce },
        `Destination token not found for ${transfer.source}`,
      );
      throw error;
    }
    this._transfers.set(transfer.nonce, transfer);

    this._log.info(`${transfer.nonce}: Transfer requested in block ${transfer.blockNumber}`);
  }

  public async updateTransferStatus(nonce: string, status: Status) {
    if (this._statuses.get(nonce) === status) return;

    this._statuses.set(nonce, status);
    this._log.debug(`${nonce}: Status changed to ${status}`);
  }

  public setCompletedTransfer(nonce: string, timestamp: Date, blockNumber: bigint, txHash: string) {
    this._completed.set(
      nonce,
      new CompletedTransfer({
        id: nonce,
        timestamp,
        destNetwork: this._network,
        srcNetwork: this._counterpartNetwork,
        blockNumber,
        txHash,
      }),
    );

    this._log.info(`${nonce}: Transfer completed at block ${blockNumber} with transaction hash ${txHash}`);
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
