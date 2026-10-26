import { Entity, Column, PrimaryColumn, Index } from 'typeorm';
import { Network } from './_network.js';
import { Status } from './_status.js';
import type { SourceReceiptAuth, ProofReference } from '../../eth/finality.js';

export type CompletionAuth = ProofReference & (
  | { version: 2; kind: 'ethereum-receipt'; checkpointId: string; completionId: string; processedLogIndex: number }
  | { version: 2; kind: 'gear-settlement'; sourceCheckpointId: string; receiptRelayId: string;
      receiptSettlementId: string; outcome: 'Completed' | 'Refunded' });

export interface LegacyReceiptAuth {
  version: 1;
  checkpointId: string;
  receipt: SourceReceiptAuth['receipt'];
  blockTimestamp: string;
  localLogIndex?: number;
}

export interface LegacyEthereumAuth extends LegacyResolution {
  sourceReceiptAuth: LegacyReceiptAuth | null;
  completionAuth: { version: 1; kind: 'ethereum-receipt' | 'gear-settlement'; checkpointId?: string;
    sourceCheckpointId?: string; completionId?: string; processedLogIndex?: number;
    receiptRelayId?: string; receiptSettlementId?: string; outcome?: 'Completed' | 'Refunded' } | null;
}

export interface LegacyResolution {
  status: Status;
  completedAt: string | null;
  completedAtBlock: string | null;
  completedAtTxHash: string | null;
}

@Entity({ name: 'transfer' })
export class Transfer {
  constructor(props?: Partial<Transfer>) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Column({ nullable: false, name: 'tx_hash' })
  txHash!: string;

  @Column('bigint', { nullable: false, name: 'block_number' })
  blockNumber!: bigint;

  @Column('bigint', { nullable: true, name: 'source_transaction_index' })
  sourceTransactionIndex?: bigint;

  @Column('bigint', { nullable: true, name: 'source_log_index' })
  sourceLogIndex?: bigint;

  @Index('IDX_transfer_receipt_slot')
  @Column('bigint', { nullable: true, name: 'receipt_slot' })
  receiptSlot?: bigint;

  @Column('jsonb', { nullable: true, name: 'source_receipt_auth' })
  sourceReceiptAuth?: SourceReceiptAuth | null;

  @Column('jsonb', { nullable: true, name: 'completion_auth' })
  completionAuth?: CompletionAuth | null;

  /** Original projection, preserved once before fail-closed invalidation. */
  @Column('jsonb', { nullable: true, name: 'legacy_resolution' })
  legacyResolution?: LegacyResolution | null;

  @Column('jsonb', { nullable: true, name: 'legacy_ethereum_auth' })
  legacyEthereumAuth?: LegacyEthereumAuth | null;

  @Index()
  @Column('timestamp with time zone', { nullable: false })
  timestamp!: Date;

  /** Finalized terminal resolution coordinates; Refunded is wrapped recovery, not native delivery. */
  @Column('timestamp with time zone', { nullable: true, name: 'completed_at' })
  completedAt?: Date;

  @Column('bigint', { nullable: true, name: 'completed_at_block' })
  completedAtBlock?: bigint;

  @Column({ nullable: true, name: 'completed_at_tx_hash' })
  completedAtTxHash?: string;

  @Index()
  @Column({ nullable: false })
  nonce!: string;

  @Column('enum', { enum: Network, nullable: false, name: 'source_network' })
  sourceNetwork!: Network;

  @Index()
  @Column({ nullable: false })
  source!: string;

  @Column('enum', { enum: Network, nullable: false, name: 'dest_network' })
  destNetwork!: Network;

  @Index()
  @Column('varchar', { nullable: true })
  destination?: string | null;

  @Column('enum', { enum: Status, nullable: false })
  status!: Status;

  @Index()
  @Column({ nullable: false })
  sender!: string;

  @Index()
  @Column({ nullable: false })
  receiver!: string;

  @Column({ nullable: false })
  amount!: string;

  @Column('bigint', { nullable: true, name: 'bridging_started_at_block' })
  bridgingStartedAtBlock?: bigint;

  @Column({ nullable: true, name: 'bridging_started_at_message_id' })
  bridgingStartedAtMessageId?: string;

  @Column('bigint', { nullable: true, name: 'eth_bridge_built_in_queue_id' })
  ethBridgeBuiltInQueueId?: bigint;

  @Column({ nullable: true, name: 'eth_bridge_built_in_msg_hash', length: 66 })
  ethBridgeBuiltInMsgHash?: `0x${string}`;

  @Column({ default: false, name: 'is_priority_fee_paid' })
  isPriorityFeePaid?: boolean;
}
