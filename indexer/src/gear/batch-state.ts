import { BlockHeader, DataHandlerContext } from '@subsquid/substrate-processor';
import { InitiatedTransfer, Network, Pair, Status, Transfer } from '../model';
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
  private _initiatedTransfers: Map<string, InitiatedTransfer>;
  private _transfersToRemove: Set<Transfer>;

  constructor() {
    super(NETWORK);
    this._addedPairs = new Map();
    this._removedPairs = new Map();
    this._upgradedPairs = new Map();
    this._initiatedTransfers = new Map();
    this._transfersToRemove = new Set();
  }

  public async new(ctx: DataHandlerContext<Store, any>) {
    await super.new(ctx);
    this._addedPairs.clear();
    this._removedPairs.clear();
    this._upgradedPairs.clear();
    this._initiatedTransfers.clear();
    const initTransfers = await ctx.store.find(InitiatedTransfer);
    for (const transfer of initTransfers) {
      this._initiatedTransfers.set(transfer.id, transfer);
    }
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
        pair.isActive = false;
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
        pair.isActive = false;
      }
      pairs.push(...upgraded);
      this._ctx.log.info(`Saved ${upgraded.length} upgraded pairs`);
    }

    if (pairs.length > 0) {
      await this._ctx.store.save(pairs);
    }
  }

  private async _saveInitiatedTransfers() {
    const transfers = Array.from(this._transfers.values());

    const initTransfers = Array.from(this._initiatedTransfers.values()).filter(
      ({ id }) => !Boolean(transfers.find((t) => t.id === id)),
    );

    const initTransfersToRemove = Array.from(this._initiatedTransfers.values()).filter(({ id }) =>
      Boolean(transfers.find((t) => t.id === id)),
    );

    await this._ctx.store.save(initTransfers);
    await this._ctx.store.remove(initTransfersToRemove);
  }

  public async save() {
    if (this._transfersToRemove.size > 0) {
      await this._ctx.store.remove(Array.from(this._transfersToRemove.values()));
    }

    await super.save();

    await this._savePairs();
    await this._saveInitiatedTransfers();
  }

  public async handleRequestBridgingReply(id: string, nonce?: string) {
    const initTransfer = this._initiatedTransfers.get(id);

    if (!initTransfer) {
      this._ctx.log.error(`Initiated transfer ${id} not found`);
      return;
    }

    if (!nonce) {
      this._ctx.log.error(`Nonce not provided for initiated transfer ${id}`);
      return;
    }

    const transfer = await this._getTransfer(nonce);

    if (!transfer) {
      this._ctx.log.error(`Transfer ${nonce} not found`);
      return;
    }

    this._transfersToRemove.add(transfer);

    this._transfers.set(
      transfer.nonce,
      new Transfer({
        ...transfer,
        id,
        txHash: initTransfer.txHash,
        blockNumber: initTransfer.blockNumber,
      }),
    );
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
      isActive: true,
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

  public async addInitiatedTransfer(transfer: InitiatedTransfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    this._initiatedTransfers.set(transfer.id, transfer);

    this._ctx.log.info(`${transfer.id}: Transfer requested in block ${transfer.blockNumber}`);
  }
}
