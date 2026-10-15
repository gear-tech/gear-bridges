import { Entity, Column, PrimaryColumn, Index } from 'typeorm';
import type { ReceiptDepositSettled } from '../../gear/types/vft-manager.js';

@Entity({ name: 'receipt_settlement' })
export class ReceiptSettlement {
  constructor(props?: Partial<ReceiptSettlement>) { Object.assign(this, props); }

  @PrimaryColumn()
  id!: string;

  @Column('jsonb')
  deposit!: ReceiptDepositSettled;

  @Column('timestamp with time zone')
  timestamp!: Date;

  @Column('bigint', { name: 'block_number' })
  blockNumber!: bigint;

  @Column({ name: 'tx_hash' })
  txHash!: string;

  @Index('IDX_receipt_settlement_matched')
  @Column({ default: false })
  matched!: boolean;
}
