import { Entity, Column, PrimaryColumn } from 'typeorm';

@Entity({ name: 'receipt_relay' })
export class ReceiptRelay {
  constructor(props?: Partial<ReceiptRelay>) { Object.assign(this, props); }

  @PrimaryColumn()
  id!: string;

  @Column('bigint', { name: 'block_number' })
  blockNumber!: bigint;
}
