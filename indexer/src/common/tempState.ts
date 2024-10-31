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

  constructor(private _network: 'eth' | 'gear') {
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

      if (transfers.length > 0) {
        transfers.forEach((t) => {
          t.status = Status.Completed;
          this._completed.delete(t.nonce);
        });
        await this._ctx.store.save(transfers);
      }
      if (this._completed.size > 0) {
        await this._ctx.store.save(Array.from(this._completed.values()));
      }
    }

    if (this._transfers.size > 0 || this._addedTokens.length > 0 || this._completed.size > 0) {
      this._ctx.log.info(
        { transfers: this._transfers.size, completed: this._completed.size, tokens: this._addedTokens.length },
        'Saved',
      );
    }
  }

  private async _getTokens() {
    const tokens = await this._ctx.store.find(Pair);

    for (const token of tokens) {
      if (this._network === 'eth') {
        this._tokens.set(token.ethToken, token);
      } else {
        this._tokens.set(token.gearToken, token);
      }
    }
  }

  private async _getCompleted() {
    const completed = await this._ctx.store.find(CompletedTransfer);

    for (const c of completed) {
      this._completed.set(c.nonce, c);
    }
  }

  public getDestinationAddress(source: string): string {
    source = source.toLowerCase();
    const pair = this._tokens.get(source);
    if (!pair) {
      return this._network === 'eth' ? ZERO_ADDRESS : ZeroAddress;
    }
    if (this._network === 'eth') {
      return pair.gearToken;
    } else {
      return pair.ethToken;
    }
  }

  public addPair(gear: string, eth: string) {
    const pair = new Pair({
      id: randomUUID(),
      gearToken: gear.toLowerCase(),
      ethToken: eth.toLowerCase(),
    });
    if (this._network === 'eth') this._tokens.set(eth, pair);
    else this._tokens.set(gear, pair);

    this._addedTokens.push(pair);

    this._ctx.log.info({ gear, eth }, 'Pair added');
  }

  public removePair(gear: string, eth: string) {
    // TODO
  }

  public transferRequested(transfer: Transfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    transfer.source = transfer.source.toLowerCase();
    transfer.destination = transfer.destination.toLowerCase();
    transfer.sender = transfer.sender.toLowerCase();
    transfer.receiver = transfer.receiver.toLowerCase();
    transfer.nonce = transfer.nonce;
    this._transfers.set(transfer.nonce, transfer);

    this._ctx.log.info(`Transfer requested :: ${transfer.nonce}`);
  }

  public transferCompleted(nonce: string) {
    this._completed.set(nonce, new CompletedTransfer({ id: randomUUID(), nonce }));
    this._ctx.log.info(`Transfer completed: ${nonce}`);
  }

  // private async _getTransfer(nonce: string): Promise<Transfer | null> {
  //   if (this._transfers.has(nonce)) {
  //     return this._transfers.get(nonce)!;
  //   }

  //   const transfer = await this._ctx.store.findOneBy(Transfer, { nonce });

  //   if (transfer) {
  //     this._transfers.set(nonce, transfer);
  //     return transfer;
  //   }

  //   return null;
  // }

  private _getTransfers(nonces: string[]) {
    return this._ctx.store.find(Transfer, {
      where: { nonce: In(nonces), destNetwork: this._network === 'gear' ? Network.Gear : Network.Ethereum },
    });
  }
}
