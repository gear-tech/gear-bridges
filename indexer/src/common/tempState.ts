import { DataHandlerContext as SContext } from '@subsquid/substrate-processor';
import { DataHandlerContext as EContext } from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';
import { ZERO_ADDRESS } from 'sails-js';
import { ZeroAddress } from 'ethers';
import { randomUUID } from 'crypto';
import { In } from 'typeorm';
import { CompletedTransfer, Network, Pair, Status, Transfer } from '../model';
import { hash } from './hash';

export class TempState {
  private _transfers: Map<string, Transfer>;
  private _completed: Map<string, CompletedTransfer>;
  private _ctx: SContext<Store, any> | EContext<Store, any>;
  private _pairs: Map<string, Pair>;
  private _addedPairs: Map<string, Pair>;
  private _removedPairs: Set<string>;

  constructor(private _network: Network) {
    this._transfers = new Map();
    this._pairs = new Map();
    this._completed = new Map();
    this._addedPairs = new Map();
    this._removedPairs = new Set();
  }

  public async new(ctx: SContext<Store, any> | EContext<Store, any>) {
    this._ctx = ctx;
    this._transfers.clear();
    this._pairs.clear();
    this._completed.clear();
    this._addedPairs.clear();
    this._removedPairs.clear();
    await this._getTokens();
    await this._getCompleted();
  }

  public async save() {
    if (this._transfers.size > 0) {
      await this._ctx.store.save(Array.from(this._transfers.values()));
    }

    if (this._addedPairs.size > 0) {
      await this._ctx.store.save(Array.from(this._addedPairs.values()));
    }

    if (this._removedPairs.size > 0) {
      const pairsToRemove = Array.from(this._pairs.values()).filter(({ id }) => this._removedPairs.has(id));
      for (const pair of pairsToRemove) {
        pair.isRemoved = true;
      }
      await this._ctx.store.save(pairsToRemove);
    }

    if (this._completed.size > 0) {
      const transfers = await this._getTransfers(Array.from(this._completed.keys()));
      const completedToDelete: CompletedTransfer[] = [];

      if (transfers.length > 0) {
        for (const t of transfers) {
          const completed = this._completed.get(t.nonce)!;
          t.status = Status.Completed;
          t.completedAt = completed.timestamp;
          completedToDelete.push(completed);
          this._completed.delete(t.nonce);
        }
        if (completedToDelete.length > 0) {
          await this._ctx.store.save(transfers);
          await this._ctx.store.remove(completedToDelete);
        }
      }
      if (this._completed.size > 0) {
        await this._ctx.store.save(Array.from(this._completed.values()));
      }
    }

    if (
      this._transfers.size > 0 ||
      this._addedPairs.size > 0 ||
      this._completed.size > 0 ||
      this._removedPairs.size > 0
    ) {
      this._ctx.log.info(
        {
          transfers: this._transfers.size > 0 ? this._transfers.size : undefined,
          completed: this._completed.size > 0 ? this._completed.size : undefined,
          addedPairs: this._addedPairs.size > 0 ? this._addedPairs.size : undefined,
          removedPairs: this._removedPairs.size > 0 ? this._removedPairs.size : undefined,
        },
        'Saved',
      );
    }
  }

  private async _getTokens() {
    const tokens = await this._ctx.store.find(Pair);

    for (const token of tokens) {
      if (this._network === Network.Ethereum) {
        this._pairs.set(token.ethToken, token);
      } else {
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

  public getDestinationAddress(source: string): string {
    source = source.toLowerCase();
    const pair = this._pairs.get(source);
    if (!pair) {
      return this._network === Network.Ethereum ? ZERO_ADDRESS : ZeroAddress;
    }
    if (this._network === Network.Ethereum) {
      return pair.varaToken;
    } else {
      return pair.ethToken;
    }
  }

  public addPair(
    varaToken: string,
    ethToken: string,
    supply: Network,
    varaTokenSymbol: string,
    ethTokenSymbol: string,
  ) {
    const vara = varaToken.toLowerCase();
    const eth = ethToken.toLowerCase();
    const id = hash(vara, eth);
    if (this._addedPairs.has(id)) {
      return;
    }
    const pair = new Pair({
      id,
      varaToken: vara,
      varaTokenSymbol,
      ethToken: eth,
      ethTokenSymbol,
      tokenSupply: supply,
      isRemoved: false,
    });
    if (this._network === Network.Ethereum) this._pairs.set(ethToken, pair);
    else this._pairs.set(varaToken, pair);

    this._addedPairs.set(id, pair);

    this._ctx.log.info({ varaToken, ethToken, varaTokenSymbol, ethTokenSymbol, supply }, 'Pair added');
  }

  public removePair(varaToken: string, ethToken: string) {
    this._removedPairs.add(hash(varaToken, ethToken));
  }

  public transferRequested(transfer: Transfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    transfer.source = transfer.source.toLowerCase();
    transfer.destination = transfer.destination.toLowerCase();
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

  public async transferStatus(nonce: string, status: Status) {
    if (this._transfers.has(nonce)) {
      this._transfers.get(nonce)!.status = status;
    } else {
      const transfer = await this._ctx.store.findOneBy(Transfer, { nonce });
      if (!transfer) {
        this._ctx.log.error(`${nonce}: Failed to update transfer status`);
        return;
      }
      transfer.status = status;
      this._transfers.set(nonce, transfer);
    }
    this._ctx.log.info(`${nonce}: Transfer changed status to ${status}`);
  }

  private _getTransfers(nonces: string[]) {
    return this._ctx.store.find(Transfer, {
      where: { nonce: In(nonces), destNetwork: this._network },
    });
  }
}
