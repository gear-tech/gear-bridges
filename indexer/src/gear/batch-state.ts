import { BlockHeader, DataHandlerContext } from '@subsquid/substrate-processor';
import { Network, Pair } from '../model';
import { Store } from '@subsquid/typeorm-store';
import { BaseBatchState, hash } from '../common';
import { In } from 'typeorm';
import {
  getEthTokenDecimals,
  getEthTokenName,
  getEthTokenSymbol,
  getProgramInheritor,
  getVaraTokenDecimals,
  getVaraTokenName,
  getVaraTokenSymbol,
} from './rpc-queries';

interface TokenMetadata {
  symbol: string;
  decimals: number;
  name: string;
}

const NETWORK = Network.Vara;

export class BatchState extends BaseBatchState<DataHandlerContext<Store, any>> {
  private _addedPairs: Map<string, Pair>;
  private _removedPairs: Map<string, bigint>;
  private _upgradedPairs: Map<
    string,
    {
      newId: string;
      activeToBlock: bigint;
    }
  >;

  constructor() {
    super(NETWORK);
    this._addedPairs = new Map();
    this._removedPairs = new Map();
    this._upgradedPairs = new Map();
  }

  public async new(ctx: DataHandlerContext<Store, any>) {
    await super.new(ctx);
    this._addedPairs.clear();
    this._removedPairs.clear();
    this._upgradedPairs.clear();
  }

  private async _savePairs() {
    const pairs: Pair[] = [];

    if (this._addedPairs.size > 0) {
      await this._ctx.store.save(Array.from(this._addedPairs.values()));
      this._ctx.log.info(`Saved ${this._addedPairs.size} new pairs`);
    }

    if (this._removedPairs.size > 0) {
      const removed = await this._ctx.store.find(Pair, {
        where: { isRemoved: false, id: In(Array.from(this._removedPairs.keys())) },
      });
      for (const pair of removed) {
        pair.isRemoved = true;
        pair.activeToBlock = this._removedPairs.get(pair.id);
      }
      pairs.push(...removed);
      this._ctx.log.info(`Saved ${removed.length} removed pairs`);
    }

    if (this._upgradedPairs.size > 0) {
      const upgraded = await this._ctx.store.find(Pair, {
        where: { isRemoved: false, id: In(Array.from(this._upgradedPairs.keys())) },
      });
      for (const pair of upgraded) {
        const { newId, activeToBlock } = this._upgradedPairs.get(pair.id)!;
        pair.upgradedTo = newId;
        pair.activeToBlock = activeToBlock;
      }
      pairs.push(...upgraded);
      this._ctx.log.info(`Saved ${upgraded.length} upgraded pairs`);
    }

    if (pairs.length > 0) {
      await this._ctx.store.save(pairs);
    }
  }

  public async save() {
    await super.save();

    await this._savePairs();
  }

  public async addPair(varaToken: string, ethToken: string, supply: Network, blockHeader: BlockHeader) {
    const vara = varaToken.toLowerCase();
    const eth = ethToken.toLowerCase();
    const id = hash(vara, eth);

    // Check if pair already exists or is being added in this block
    const existingPair = this._pairs.get(vara);
    const addedPair = this._addedPairs.get(id);

    if (existingPair && !existingPair.isRemoved && !existingPair.upgradedTo) {
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
      activeSinceBlock: BigInt(blockHeader.height),
    });

    this._pairs.set(vara, pair);

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

  public removePair(varaToken: string, ethToken: string, blockNumber: bigint) {
    const vftAddr = varaToken.toLowerCase();
    const erc20Addr = ethToken.toLowerCase();
    this._removedPairs.set(hash(vftAddr, erc20Addr), blockNumber);
    this._ctx.log.info(
      {
        vftAddr,
        erc20Addr,
      },
      'Pair removed',
    );
  }

  public async upgradePair(varaToken: string, block: BlockHeader) {
    const vftAddr = varaToken.toLowerCase();

    if (!this._pairs.has(vftAddr)) return;

    const pair = this._pairs.get(vftAddr)!;

    const newId = await getProgramInheritor(this._ctx._chain.rpc, block._runtime, vftAddr, block.hash);

    this._upgradedPairs.set(pair.id, {
      newId,
      activeToBlock: BigInt(block.height),
    });

    await this.addPair(vftAddr, pair.ethToken, pair.tokenSupply, block);

    this._ctx.log.info(`Vara Token ${vftAddr} upgraded to ${newId}`);
  }

  private async _fetchVaraMetadata(varaTokenId: string, blockHeader: BlockHeader): Promise<TokenMetadata> {
    const rpc = this._ctx._chain.rpc;
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

  public getActiveVaraTokens(): string[] {
    const activePairs = Array.from(this._pairs.values())
      .filter(({ isRemoved, upgradedTo }) => !isRemoved && !upgradedTo)
      .map(({ varaToken }) => varaToken);

    return activePairs;
  }
}
