import { DataHandlerContext as SContext } from '@subsquid/substrate-processor';
import { DataHandlerContext as EContext } from '@subsquid/evm-processor';
import { Store } from '@subsquid/typeorm-store';
import { ZERO_ADDRESS } from 'sails-js';
import { ZeroAddress } from 'ethers';
import { randomUUID } from 'crypto';
import { In } from 'typeorm';
import { CompletedTransfer, Network, Pair, Status, Transfer } from '../model';

export class TempState {
  private _transfers: Map<string, Transfer>;
  private _completed: Map<string, CompletedTransfer>;
  private _ctx: SContext<Store, any> | EContext<Store, any>;
  private _tokens: Map<string, Pair>;
  private _addedTokens: Array<Pair>;

  constructor(private _network: Network) {
    this._transfers = new Map();
    this._tokens = new Map();
    this._completed = new Map();
  }

  public async new(ctx: SContext<Store, any> | EContext<Store, any>) {
    this._ctx = ctx;
    this._transfers.clear();
    this._tokens.clear();
    this._completed.clear();
    this._addedTokens = [];
    await this._getTokens();
    await this._getCompleted();
  }

  public async save() {
    if (this._transfers.size > 0) {
      await this._ctx.store.save(Array.from(this._transfers.values()));
    }

    if (this._addedTokens.length > 0) {
      await this._ctx.store.save(this._addedTokens);
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

    if (this._transfers.size > 0 || this._addedTokens.length > 0 || this._completed.size > 0) {
      this._ctx.log.info(
        `Saved: ${this._transfers.size} transfers, ${this._completed.size} completed, ${this._addedTokens.length} pairs`,
      );
    }
  }

  private async _getTokens() {
    const tokens = await this._ctx.store.find(Pair);

    for (const token of tokens) {
      if (this._network === Network.Ethereum) {
        this._tokens.set(token.ethToken, token);
      } else {
        this._tokens.set(token.gearToken, token);
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
    const pair = this._tokens.get(source);
    if (!pair) {
      return this._network === Network.Ethereum ? ZERO_ADDRESS : ZeroAddress;
    }
    if (this._network === Network.Ethereum) {
      return pair.gearToken;
    } else {
      return pair.ethToken;
    }
  }

  public addPair(gear: string, eth: string, supply: Network) {
    const pair = new Pair({
      id: randomUUID(),
      gearToken: gear.toLowerCase(),
      ethToken: eth.toLowerCase(),
      tokenSupply: supply,
    });
    if (this._network === Network.Ethereum) this._tokens.set(eth, pair);
    else this._tokens.set(gear, pair);

    this._addedTokens.push(pair);

    this._ctx.log.info({ gear, eth, supply }, 'Pair added');
  }

  public async removePair(gear: string, eth: string) {
    const index = this._addedTokens.findIndex(({ gearToken, ethToken }) => gearToken === gear && ethToken === eth);
    if (index > 0) {
      this._addedTokens.splice(index);
    } else {
      const pair = await this._ctx.store.findOneBy(Pair, { gearToken: gear, ethToken: eth });
      if (!pair) return;

      await this._ctx.store.remove(pair);
    }
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
      transfer.status = Status.InProgress;
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
