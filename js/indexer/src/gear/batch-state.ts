import { BlockHeader, DataHandlerContext } from '@subsquid/substrate-processor';
import { Store } from '@subsquid/typeorm-store';
import { createPairHash } from 'gear-bridge-common';
import { In, Raw } from 'typeorm';

import { GearEthBridgeMessage, InitiatedTransfer, Network, Pair, Transfer, ReceiptRelay, ReceiptSettlement } from '../model/index.js';
import { BaseBatchState, mapKeys, mapValues, setValues } from '../common/index.js';
import {
  getEthTokenDecimals,
  getEthTokenName,
  getEthTokenSymbol,
  getProgramInheritor,
  getVaraTokenDecimals,
  getVaraTokenName,
  getVaraTokenSymbol,
} from './rpc-queries.js';
import type { Relayed, ReceiptDepositEffect, ReceiptDepositRefunded, ReceiptDepositSettled } from './types/index.js';
import { createEthereumEvidenceClients, createReceiptAuthenticator, qualifyStoredFinality, recheckFinality, retainCheckpoint,
  type SourceReceiptAuth } from '../eth/finality.js';
import { authenticateSourceReceipt } from '../eth/completion.js';
import { reconcileReceiptSettlements, validateReceiptSettlement } from './receipt-reconciliation.js';
import { config } from './config.js';
import { updateId } from './programIds.js';
import { ProgramName } from './util.js';

function sameDeposit(a: ReceiptDepositEffect, b: ReceiptDepositEffect): boolean {
  return Object.keys(a).length === Object.keys(b).length &&
    Object.entries(a).every(([key, value]) => String(value) === String(b[key as keyof ReceiptDepositSettled]));
}

interface TokenMetadata {
  symbol: string;
  decimals: number;
  name: string;
}

const NETWORK = Network.Vara;
const COUNTERPART_NETWORK = Network.Ethereum;

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
  private _ethBridgeMessages: Map<string, GearEthBridgeMessage>;
  private _receiptRelays: Map<string, Relayed> = new Map();
  private _receiptSettlements: Map<string, ReceiptSettlement> = new Map();
  private _receiptSettlementCursor?: string;
  private _receiptSettlementBatch?: string;
  private _receiptSettlementStart?: string;

  constructor() {
    super(NETWORK, COUNTERPART_NETWORK);
    this._addedPairs = new Map();
    this._removedPairs = new Map();
    this._upgradedPairs = new Map();
    this._initiatedTransfers = new Map();
    this._transfersToRemove = new Set();
    this._ethBridgeMessages = new Map();
  }

  protected _clear(): void {
    super._clear();
    this._addedPairs.clear();
    this._removedPairs.clear();
    this._upgradedPairs.clear();
    this._initiatedTransfers.clear();
    this._ethBridgeMessages.clear();
    this._receiptRelays.clear();
    this._receiptSettlements.clear();
  }

  public async new(ctx: DataHandlerContext<Store, any>) {
    const batchKey = ctx.blocks.map(({ header }) => `${header.height}:${header.hash}`).join(',');
    if (this._receiptSettlementBatch === batchKey) {
      this._receiptSettlementCursor = this._receiptSettlementStart;
    } else {
      this._receiptSettlementBatch = batchKey;
      this._receiptSettlementStart = this._receiptSettlementCursor;
    }
    await super.new(ctx);
    const initTransfers = await ctx.store.find(InitiatedTransfer, {});
    for (const transfer of initTransfers) {
      this._initiatedTransfers.set(transfer.id, transfer);
    }
  }
  public async updateProgramId(name: ProgramName, programId: string) {
    await updateId(this._ctx.store, name, programId);
  }


  private async _savePairs() {
    const pairs: Pair[] = [];

    if (this._addedPairs.size > 0) {
      await this._ctx.store.save(mapValues(this._addedPairs));
      this._log.info({ count: this._addedPairs.size }, 'New pairs saved');
    }

    if (this._removedPairs.size > 0) {
      const removed = await this._ctx.store.find(Pair, {
        where: { isRemoved: false, id: In(mapKeys(this._removedPairs)) },
      });
      for (const pair of removed) {
        pair.isRemoved = true;
        pair.activeToBlock = this._removedPairs.get(pair.id);
        pair.isActive = false;
      }
      pairs.push(...removed);
      this._log.info({ count: removed.length }, 'Removed pairs saved');
    }

    if (this._upgradedPairs.size > 0) {
      const upgraded = await this._ctx.store.find(Pair, {
        where: { isRemoved: false, id: In(mapKeys(this._upgradedPairs)) },
      });
      for (const pair of upgraded) {
        const { newId, activeToBlock } = this._upgradedPairs.get(pair.id)!;
        pair.upgradedTo = newId;
        pair.activeToBlock = activeToBlock;
        pair.isActive = false;
      }
      pairs.push(...upgraded);
      this._log.info({ count: upgraded.length }, 'Upgraded pairs saved');
    }

    if (pairs.length > 0) {
      await this._ctx.store.save(pairs);
    }
  }

  private async _saveInitiatedTransfers() {
    const transfers = mapValues(this._transfers);

    const initTransfers = mapValues(this._initiatedTransfers).filter(({ id }) => !transfers.find((t) => t.id === id));

    const initTransfersToRemove = mapValues(this._initiatedTransfers).filter(({ id }) =>
      Boolean(transfers.find((t) => t.id === id)),
    );

    await this._ctx.store.save(initTransfers);
    await this._ctx.store.remove(initTransfersToRemove);
  }

  private async _saveEthBridgeMessages() {
    if (this._ethBridgeMessages.size === 0) return;

    for (const [nonce, message] of this._ethBridgeMessages.entries()) {
      if (this._transfers.has(nonce)) {
        const transfer = this._transfers.get(nonce)!;

        transfer.bridgingStartedAtBlock = message.blockNumber;
        transfer.bridgingStartedAtMessageId = message.id;
      }
    }
    await this._ctx.store.save(mapValues(this._ethBridgeMessages));

    this._log.info({ count: this._ethBridgeMessages.size }, 'Gear ETH bridge messages saved');
  }

  protected async _saveTransfers(): Promise<void> {
    const nonces: string[] = [];

    for (const [nonce, transfer] of this._transfers.entries()) {
      if (transfer.bridgingStartedAtBlock && transfer.bridgingStartedAtMessageId) {
        continue;
      }

      nonces.push(nonce);
    }

    const ethBridgeMessages = await this._ctx.store.find(GearEthBridgeMessage, {
      where: { nonce: In(nonces) },
    });

    for (const { nonce, blockNumber, id } of ethBridgeMessages) {
      const transfer = this._transfers.get(nonce)!;

      transfer.bridgingStartedAtBlock = blockNumber;
      transfer.bridgingStartedAtMessageId = id;
    }

    await super._saveTransfers();
  }

  public recordReceiptRelay(relay: Relayed) {
    const id = `${relay.slot}:${relay.transaction_index}`;
    const existing = this._receiptRelays.get(id);
    if (existing && existing.block_number !== relay.block_number) throw new Error('HOLD: conflicting authenticated receipt block');
    this._receiptRelays.set(id, relay);
  }

  public recordReceiptSettlement(deposit: ReceiptDepositEffect, timestamp: Date, blockNumber: bigint, txHash: string) {
    validateReceiptSettlement(deposit);
    const id = `${deposit.slot}:${deposit.transaction_index}:${deposit.log_index}`;
    const existing = this._receiptSettlements.get(id);
    if (existing && !sameDeposit(existing.deposit, deposit)) throw new Error('HOLD: conflicting receipt deposit settlement');
    if (!existing) this._receiptSettlements.set(id, new ReceiptSettlement({ id, deposit, timestamp, blockNumber, txHash, matched: false }));
  }

  public recordReceiptRefund(deposit: ReceiptDepositRefunded, timestamp: Date, blockNumber: bigint, txHash: string) {
    this.recordReceiptSettlement({ ...deposit, outcome: 'Refunded' }, timestamp, blockNumber, txHash);
  }

  private async _saveReceiptSettlements(authenticateSource: (transfer: Transfer) => Promise<SourceReceiptAuth | undefined>) {
    const batchSize = 1000;
    const relays = [...this._receiptRelays.entries()];
    for (let offset = 0; offset < relays.length; offset += batchSize) {
      const batch = relays.slice(offset, offset + batchSize);
      const existing = new Map((await this._ctx.store.find(ReceiptRelay, {
        where: { id: In(batch.map(([id]) => id)) },
      })).map((row) => [row.id, row]));
      const added: ReceiptRelay[] = [];
      for (const [id, relay] of batch) {
        const row = existing.get(id);
        if (row && BigInt(row.blockNumber) !== BigInt(relay.block_number)) {
          throw new Error('HOLD: conflicting authenticated receipt block');
        }
        if (!row) added.push(new ReceiptRelay({ id, blockNumber: BigInt(relay.block_number) }));
      }
      if (added.length) await this._ctx.store.save(added);
    }
    const settlements = mapValues(this._receiptSettlements);
    for (let offset = 0; offset < settlements.length; offset += batchSize) {
      const batch = settlements.slice(offset, offset + batchSize);
      const existing = new Map((await this._ctx.store.find(ReceiptSettlement, {
        where: { id: In(batch.map(({ id }) => id)) },
      })).map((row) => [row.id, row]));
      const added: ReceiptSettlement[] = [];
      for (const settlement of batch) {
        const row = existing.get(settlement.id);
        if (row && !sameDeposit(row.deposit, settlement.deposit)) {
          throw new Error('HOLD: conflicting receipt deposit settlement');
        }
        if (!row) added.push(settlement);
      }
      if (added.length) await this._ctx.store.save(added);
    }
    // Repair owns migrated, unauthenticated settlements. Filter before LIMIT so
    // unavailable historical receipts cannot starve unrelated live ingestion.
    const loadPending = () => this._ctx.store.find(ReceiptSettlement, {
      where: { matched: false, id: Raw((alias) => {
        const id = alias.split('.').map((part) => `"${part.replaceAll('"', '""')}"`).join('.');
        return `
        ${this._receiptSettlementCursor ? `${id} > :settlementCursor AND` : ''}
        EXISTS (SELECT 1 FROM receipt_settlement s WHERE s.id = ${id}
          AND (s.legacy_matched IS NULL OR EXISTS (
            SELECT 1 FROM receipt_relay r JOIN transfer t
              ON t.source_network = 'Ethereum' AND t.block_number = r.block_number
              AND t.source_transaction_index = (s.deposit->>'transaction_index')::numeric
              AND t.source_log_index = (s.deposit->>'log_index')::numeric
            WHERE r.id = (s.deposit->>'slot') || ':' || (s.deposit->>'transaction_index')
              AND t.source_receipt_auth IS NOT NULL)))`;
      }, { settlementCursor: this._receiptSettlementCursor }) },
      order: { id: 'ASC' }, take: batchSize,
    });
    let pending = await loadPending();
    if (!pending.length && this._receiptSettlementCursor) {
      this._receiptSettlementCursor = undefined;
      pending = await loadPending();
    }
    if (!pending.length) return;
    const updates = await reconcileReceiptSettlements(this._ctx.store, pending, authenticateSource);
    if (updates.transfers.length) await this._ctx.store.save(updates.transfers);
    if (updates.settlements.length) await this._ctx.store.save(updates.settlements);
    this._receiptSettlementCursor = pending[pending.length - 1].id;
  }

  public async save() {
    const clients = createEthereumEvidenceClients('gear');
    const checkpoint = await qualifyStoredFinality(this._ctx.store, clients);
    const authenticate = createReceiptAuthenticator(clients, checkpoint);
    await recheckFinality(clients, checkpoint);
    if (this._transfersToRemove.size > 0) {
      await this._ctx.store.remove(setValues(this._transfersToRemove));
    }

    await this._saveReceiptSettlements((transfer) =>
      authenticateSourceReceipt(clients, transfer, config.erc20Manager, checkpoint, authenticate));
    await Promise.all([this._saveEthBridgeMessages(), this._savePairs()]);
    await this._processStatuses();
    await this._processPriorityRequests();
    await this._saveTransfers();
    await this._saveInitiatedTransfers();
    await recheckFinality(clients, checkpoint);
    await retainCheckpoint(this._ctx.store, checkpoint);
  }

  public async handleRequestBridgingReply(id: string, nonce: string) {
    const initTransfer = this._initiatedTransfers.get(id);

    if (!initTransfer) {
      this._log.error({ transferId: id }, 'Initiated transfer not found');
      return;
    }

    if (!nonce) {
      this._log.error({ transferId: id }, 'Nonce not provided for initiated transfer');
      return;
    }

    const transfer = await this._getTransfer(nonce);

    if (!transfer) {
      this._log.error({ nonce }, 'Transfer not found');
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
    const id = createPairHash(vara, eth);

    if (supply !== Network.Ethereum && supply !== Network.Vara) throw new Error('HOLD: invalid pair supply');
    for (const pair of this._pairs.values()) {
      if (!pair.isActive || this._removedPairs.has(pair.id) || this._upgradedPairs.has(pair.id)) continue;
      if (pair.varaToken === vara || pair.ethToken === eth) {
        if (pair.id !== id || pair.tokenSupply !== supply) throw new Error('HOLD: conflicting active token pair or supply');
        return;
      }
    }

    // Fetch metadata
    let vftMetadata: TokenMetadata;
    let ercMetadata: TokenMetadata;

    try {
      vftMetadata = await this._fetchVaraMetadata(varaToken, blockHeader);
      ercMetadata = await this._fetchEthMetadata(ethToken);
    } catch (error) {
      this._log.error(
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

    this._log.info(
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
    this._removedPairs.set(createPairHash(vftAddr, erc20Addr), blockNumber);
    this._pairs.delete(vftAddr);
    this._log.info(
      {
        varaToken: vftAddr,
        ethToken: erc20Addr,
      },
      'Pair removed',
    );
  }

  public removePairById(pairId: string, blockNumber: bigint) {
    this._removedPairs.set(pairId, blockNumber);
    for (const [token, pair] of this._pairs) if (pair.id === pairId) this._pairs.delete(token);
    this._log.info(
      {
        pairId,
      },
      'Pair removed by ID',
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
    this._pairs.delete(vftAddr);

    await this.addPair(newId, pair.ethToken, pair.tokenSupply, block);

    this._log.info({ varaToken: vftAddr, newProgramId: newId }, 'Vara token upgraded');
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
    const activePairs = mapValues(this._pairs)
      .filter(({ isRemoved, upgradedTo }) => !isRemoved && !upgradedTo)
      .map(({ varaToken }) => varaToken);

    return activePairs;
  }

  public async addInitiatedTransfer(transfer: InitiatedTransfer) {
    transfer.txHash = transfer.txHash.toLowerCase();
    this._initiatedTransfers.set(transfer.id, transfer);

    this._log.info({ transferId: transfer.id, blockNumber: transfer.blockNumber }, 'Initiated transfer requested');
  }

  public addEthBridgeMessage(message: GearEthBridgeMessage) {
    this._ethBridgeMessages.set(message.nonce, message);
    this._log.info({ nonce: message.nonce, messageId: message.id }, 'Gear ETH bridge message added');
  }
}
