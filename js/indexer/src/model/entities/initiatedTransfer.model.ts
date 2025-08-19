import { Entity, Column, PrimaryColumn } from 'typeorm';

@Entity({ name: 'initiated_transfer' })
export class InitiatedTransfer {
  constructor(props?: Partial<InitiatedTransfer>) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Column({ nullable: false, name: 'tx_hash' })
  txHash!: string;

  @Column('bigint', { nullable: false, name: 'block_number' })
  blockNumber!: bigint;
}
