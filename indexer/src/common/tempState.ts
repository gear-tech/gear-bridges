import { BlockHeader, DataHandlerContext as SContext } from '@subsquid/substrate-processor';
import { DataHandlerContext as EContext } from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';
import { randomUUID } from 'crypto';
import { In } from 'typeorm';
import { CompletedTransfer, Network, Pair, Status, Transfer } from '../model';
import { hash } from './hash';
import {
  getEthTokenDecimals,
  getEthTokenName,
  getEthTokenSymbol,
  getVaraTokenDecimals,
  getVaraTokenName,
  getVaraTokenSymbol,
} from '../gear/rpc-queries';

interface TokenMetadata {
  symbol: string;
  decimals: number;
  name: string;
}

export class TempState {
  private _transfers: Map<string, Transfer>;
  private _completed: Map<string, CompletedTransfer>;
  private _ctx!: SContext<Store, any> | EContext<Store, any>; // Will be set in .new()
  private _pairs: Map<string, Pair>;
  private _addedPairs: Map<string, Pair>;
  private _removedPairs: Set<string>;
  private _statuses: Map<string, Status>;

  constructor(private _network: Network) {
    this._transfers = new Map();
    this._pairs = new Map();
    this._completed = new Map();
    this._addedPairs = new Map();
    this._removedPairs = new Set();
    this._statuses = new Map();
  }

  public async new(ctx: SContext<Store, any> | EContext<Store, any>) {
    this._ctx = ctx;
    this._transfers.clear();
    this._pairs.clear();
    this._completed.clear();
    this._addedPairs.clear();
    this._removedPairs.clear();
    this._statuses.clear();
    await this._getPairs();
    await this._getCompleted();
  }

  private async _fetchVaraMetadata(varaTokenId: string, blockHeader: BlockHeader): Promise<TokenMetadata> {
    const rpc = (this._ctx as SContext<Store, any>)._chain.rpc;
    const blockhash = blockHeader.hash;

    const [symbol, decimals, name] = await Promise.all([
      getVaraTokenSymbol(rpc, varaTokenId, blockhash),
      getVaraTokenDecimals(rpc, varaTokenId, blockhash),
      getVaraTokenName(rpc, varaTokenId, blockhash),
    ]);

    return { symbol, decimals, name };
  }

  private async _fetchEthMetadata(
    ethTokenAddress: string,
  ): Promise<{ symbol: string; decimals: number; name: string }> {
    const [symbol, decimals, name] = await Promise.all([
      getEthTokenSymbol(ethTokenAddress),
      getEthTokenDecimals(ethTokenAddress),
      getEthTokenName(ethTokenAddress),
    ]);

    return { symbol, decimals, name };
  }

  public async save() {
    try {
      const saveOperations: Promise<any>[] = [];

      await this._saveStatusUpdates();

      if (this._transfers.size > 0) {
        saveOperations.push(this._ctx.store.save(Array.from(this._transfers.values())));
      }

      await this._savePairs(saveOperations);

      if (saveOperations.length > 0) {
        await Promise.all(saveOperations);
      }

      await this._saveCompletedTransfers();

      this._logSaveOperations();
    } catch (error) {
      this._ctx.log.error({ error: error instanceof Error ? error.message : String(error) }, 'Error saving state');
      throw error;
    }
  }

  private async _savePairs(saveOperations: Promise<any>[]) {
    const pairs: Pair[] = [];

    if (this._removedPairs.size > 0) {
      const pairsToRemove = Array.from(this._pairs.values()).filter(({ id }) => this._removedPairs.has(id));
      for (const pair of pairsToRemove) {
        pair.isRemoved = true;
      }
      pairs.push(...pairsToRemove);
    }

    if (this._addedPairs.size > 0) {
      pairs.push(...Array.from(this._addedPairs.values()).filter(({ id }) => !this._removedPairs.has(id)));
    }

    if (pairs.length > 0) {
      saveOperations.push(this._ctx.store.save(pairs));
    }
  }

  private async _saveCompletedTransfers() {
    if (this._completed.size === 0) return;

    const transfers = await this._getTransfers(Array.from(this._completed.keys()));
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

      const operations: Promise<any>[] = [];
      if (transfersToUpdate.length > 0) {
        operations.push(this._ctx.store.save(transfersToUpdate));
      }
      if (completedToDelete.length > 0) {
        operations.push(this._ctx.store.remove(completedToDelete));
      }

      await Promise.all(operations);
    }

    if (this._completed.size > 0) {
      await this._ctx.store.save(Array.from(this._completed.values()));
    }
  }

  private _logSaveOperations() {
    if (
      this._transfers.size > 0 ||
      this._addedPairs.size > 0 ||
      this._completed.size > 0 ||
      this._removedPairs.size > 0
    ) {
      const logInfo: Record<string, number> = {};

      if (this._transfers.size > 0) logInfo.transfers = this._transfers.size;
      if (this._completed.size > 0) logInfo.completed = this._completed.size;
      if (this._addedPairs.size > 0) logInfo.addedPairs = this._addedPairs.size;
      if (this._removedPairs.size > 0) logInfo.removedPairs = this._removedPairs.size;

      this._ctx.log.info(logInfo, 'Saved');
    }
  }

  private async _getPairs(withRemoved = false) {
    const tokens = withRemoved
      ? await this._ctx.store.find(Pair)
      : await this._ctx.store.find(Pair, { where: { isRemoved: false } });

    for (const token of tokens) {
      if (this._network === Network.Ethereum) {
        this._ctx.log.info(`Setting pair for ${token.ethToken}`);
        this._pairs.set(token.ethToken, token);
      } else {
        this._ctx.log.info(`Setting pair for ${token.varaToken}`);
        this._pairs.set(token.varaToken, token);
      }
    }
  }

  private async _getCompleted() {
    const completed = await this._ctx.store.find(CompletedTransfer, {
      where: { destNetwork: this._network },
    });

    for (const c of completed) {
      this._completed.set(c.nonce, c);
    }
  }

  private async _getDestinationAddress(source: string): Promise<string> {
    const _source = source.toLowerCase();
    let pair = this._pairs.get(_source);
    while (!pair) {
      this._ctx.log.warn(`Pair not found for ${_source}, retrying...`);
      await new Promise((resolve) => setTimeout(resolve, 1000));
      await this._getPairs(true);
      pair = this._pairs.get(_source);
    }

    if (this._network === Network.Ethereum) {
      return pair.varaToken.toLowerCase();
    } else {
      return pair.ethToken.toLowerCase();
    }
  }

  public async addPair(varaToken: string, ethToken: string, supply: Network, blockHeader: BlockHeader) {
    const vara = varaToken.toLowerCase();
    const eth = ethToken.toLowerCase();
    const id = hash(vara, eth);

    // Check if pair already exists or is being added in this block
    const existingPair = this._pairs.get(this._network === Network.Ethereum ? eth : vara);
    const addedPair = this._addedPairs.get(id);

    if (existingPair && !existingPair.isRemoved) {
      this._ctx.log.info({ varaToken, ethToken }, 'Pair already exists, skipping addition');
      return;
    }

    if (addedPair) {
      this._ctx.log.info({ varaToken, ethToken }, 'Pair already being added in this batch, skipping addition');
      return;
    }

    // Fetch metadata
    let vftMetadata: TokenMetadata;
    let ercMetadata: TokenMetadata;

    try {
      vftMetadata = await this._fetchVaraMetadata(varaToken, blockHeader);
      ercMetadata = await this._fetchEthMetadata(ethToken);
    } catch (error) {
      this._ctx.log.error(
        { varaToken, ethToken, error: error instanceof Error ? error.message : String(error) },
        'Failed to fetch token metadata',
      );
      throw new Error('Failed to fetch token metadata');
    }

    const pair = new Pair({
      id,
      varaToken: vara,
      varaTokenSymbol: vftMetadata.symbol,
      varaTokenDecimals: vftMetadata.decimals,
      varaTokenName: vftMetadata.name,
      ethToken: eth,
      ethTokenSymbol: ercMetadata.symbol,
      ethTokenDecimals: ercMetadata.decimals,
      ethTokenName: ercMetadata.name,
      tokenSupply: supply,
      isRemoved: false,
    });

    if (this._network === Network.Ethereum) this._pairs.set(ethToken, pair);
    else this._pairs.set(varaToken, pair);

    this._addedPairs.set(id, pair);

    this._ctx.log.info(
      {
        varaToken,
        ethToken,
        vft: {
          symbol: vftMetadata.symbol,
          decimals: vftMetadata.decimals,
          name: vftMetadata.name,
        },
        erc: {
          symbol: ercMetadata.symbol,
          decimals: ercMetadata.decimals,
          name: ercMetadata.name,
        },
      },
      'Pair added',
    );
  }

  public removePair(varaToken: string, ethToken: string) {
    this._removedPairs.add(hash(varaToken, ethToken));
    const pair = this._pairs.get(varaToken);
    if (pair) {
      pair.isRemoved = true;
    }
    this._ctx.log.info(
      {
        varaToken,
        ethToken,
      },
      'Pair removed',
    );
  }

  public async transferRequested(transfer: Transfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    transfer.source = transfer.source.toLowerCase();
    transfer.destination = await this._getDestinationAddress(transfer.source);
    transfer.sender = transfer.sender.toLowerCase();
    transfer.receiver = transfer.receiver.toLowerCase();
    transfer.nonce = transfer.nonce;
    this._transfers.set(transfer.nonce, transfer);

    this._ctx.log.info(`${transfer.nonce}: Transfer requested in block ${transfer.blockNumber}`);
  }

  public transferCompleted(nonce: string, ts: Date) {
    this._completed.set(
      nonce,
      new CompletedTransfer({
        id: randomUUID(),
        nonce,
        destNetwork: this._network,
        timestamp: ts,
      }),
    );
    this._ctx.log.info(`${nonce}: Transfer completed`);
  }

  public transferStatus(nonce: string, status: Status) {
    if (this._statuses.get(nonce) === status) {
      return;
    }
    this._statuses.set(nonce, status);
    this._ctx.log.info(`${nonce}: Status changed to ${status}`);
  }

  private async _saveStatusUpdates(): Promise<void> {
    if (this._statuses.size === 0) {
      return;
    }

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

  private _getTransfers(nonces: string[]) {
    return this._ctx.store.find(Transfer, {
      where: { nonce: In(nonces), destNetwork: this._network },
    });
  }
}
