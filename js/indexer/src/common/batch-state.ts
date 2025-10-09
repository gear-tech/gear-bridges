import { DataHandlerContext as SubstrateContext } from '@subsquid/substrate-processor';
import { DataHandlerContext as EthereumContext } from '@subsquid/evm-processor';
import { In, IsNull, LessThanOrEqual, MoreThanOrEqual, Not } from 'typeorm';
import { FindManyOptions, Store } from '@subsquid/typeorm-store';
import { Logger } from '@subsquid/logger';

import { CompletedTransfer, Network, Pair, Status, Transfer } from '../model/index.js';
import { mapKeys, mapValues, setValues } from './map.js';

const PAIR_RETRY_LIMIT = 5;
const PAIR_RETRY_DELAY = 1000;

export abstract class BaseBatchState<Context extends SubstrateContext<Store, any> | EthereumContext<Store, any>> {
  protected _transfers: Map<string, Transfer>;
  protected _completed: Map<string, CompletedTransfer>;
  protected _pairs: Map<string, Pair>;
  protected _statuses: Map<string, Status>;
  protected _priorityRequests: Set<string>;
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
    this._priorityRequests = new Set();
  }

  protected _clear() {
    this._transfers.clear();
    this._completed.clear();
    this._pairs.clear();
    this._statuses.clear();
    this._priorityRequests.clear();
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
      retryCount++;
      this._log.error({ token: src, attempt: retryCount, maxAttempts: PAIR_RETRY_LIMIT }, 'Pair not found, retrying');
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
      if (retryCount >= PAIR_RETRY_LIMIT) {
        this._log.error({ token: src, attempts: retryCount }, 'Failed to load pair after max retries');
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
    this._log.info({ count: transfers.length }, 'Transfers marked as completed');
    this._log.debug({ nonces: transfers.map((transfer) => transfer.nonce) });

    await this._ctx.store.remove(completedToRemove);
    this._log.info({ count: completedToRemove.length }, 'Completed records removed');
    this._log.debug({ nonces: completedToRemove.map((transfer) => transfer.id) });
  }

  protected async _saveCompletedTransfers(): Promise<void> {
    if (this._completed.size === 0) return;

    const duplicates = await this._ctx.store.find(CompletedTransfer, {
      where: { id: In(mapKeys(this._completed)) },
    });

    const nonces = duplicates.map((info) => info.id);

    if (duplicates.length > 0) {
      this._log.info({ count: duplicates.length }, 'Found duplicates of completed transfers');

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
    this._log.info({ count: completedToSave.length }, 'Completed records saved');
    this._log.debug({ nonces: completedToSave.map((info) => info.id) });
  }

  protected async _processStatuses() {
    if (this._statuses.size === 0) return;

    const noncesToUpdate = mapKeys(this._statuses);

    await this._updateCachedTransfers(noncesToUpdate);

    const notUpdatedTransfers: string[] = [];

    for (const [nonce, status] of this._statuses.entries()) {
      const transfer = this._transfers.get(nonce);
      if (transfer) {
        if (transfer.status !== Status.Completed) {
          transfer.status = status;
        }
      } else {
        notUpdatedTransfers.push(nonce);
      }
    }

    if (notUpdatedTransfers.length > 0) {
      const transfers = await this._ctx.store.find(Transfer, {
        where: { nonce: In(notUpdatedTransfers), sourceNetwork: this._network },
      });

      const notCompletedTransfers = transfers.filter(({ status }) => status !== Status.Completed);

      for (const t of notCompletedTransfers) {
        this._log.error(
          { nonce: t.nonce, targetStatus: this._statuses.get(t.nonce) },
          'Failed to update transfer status',
        );
      }

      if (transfers.length === notUpdatedTransfers.length) return;

      const noncesInDb = transfers.map(({ nonce }) => nonce);

      const missingNonces = notUpdatedTransfers.filter((nonce) => !noncesInDb.includes(nonce));

      this._log.error({ nonces: missingNonces }, 'Nonces not found in DB or cache');
    }
  }

  protected async _processPriorityRequests() {
    if (this._priorityRequests.size === 0) return;

    const noncesToUpdate = setValues(this._priorityRequests);

    await this._updateCachedTransfers(noncesToUpdate);

    for (const nonce of noncesToUpdate) {
      const transfer = this._transfers.get(nonce);

      if (!transfer) {
        this._log.error({ nonce }, 'Transfer not found for updating priority status');
        continue;
      }

      transfer.isPriorityFeePaid = true;
    }
  }

  protected async _saveTransfers() {
    if (this._transfers.size === 0) return;

    const transfers = mapValues(this._transfers);

    for (let i = 0; i < this._transfers.size; i += 1000) {
      await this._ctx.store.save(transfers.slice(i, i + 1000));
    }

    this._log.info({ count: this._transfers.size }, 'Transfers saved');
  }

  public abstract save(): Promise<void>;

  protected async _updateCachedTransfers(nonces: string[]) {
    if (nonces.length === 0) return;

    const notCachedNonces = nonces.filter((nonce) => !this._transfers.has(nonce));

    if (notCachedNonces.length > 0) {
      const transfers = await this._ctx.store.find(Transfer, {
        where: { nonce: In(notCachedNonces), sourceNetwork: this._network, status: Not(Status.Completed) },
      });
      for (const transfer of transfers) {
        this._transfers.set(transfer.nonce, transfer);
      }
    }
  }

  public async addTransfer(transfer: Transfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    transfer.source = transfer.source.toLowerCase();
    transfer.sender = transfer.sender.toLowerCase();
    transfer.receiver = transfer.receiver.toLowerCase();
    try {
      transfer.destination = await this._getDestinationAddress(transfer.source);
    } catch (error) {
      this._log.error(
        {
          nonce: transfer.nonce,
          blockNumber: transfer.blockNumber,
          txHash: transfer.txHash,
          sourceToken: transfer.source,
        },
        'Destination token not found for source token',
      );
      throw error;
    }
    this._transfers.set(transfer.nonce, transfer);

    this._log.info({ nonce: transfer.nonce, blockNumber: transfer.blockNumber }, 'Transfer requested');
  }

  public async updateTransferStatus(nonce: string, status: Status, isPriority = false) {
    if (this._statuses.get(nonce) === status) return;

    this._statuses.set(nonce, status);
    if (isPriority) {
      this._priorityRequests.add(nonce);
      this._log.info({ nonce }, 'Request marked as priority');
    }

    this._log.debug({ nonce, status }, 'Request status changed');
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

    this._log.info({ nonce, blockNumber, txHash }, 'Transfer completed');
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
