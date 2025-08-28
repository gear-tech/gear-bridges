import { Entity, Column, PrimaryColumn } from 'typeorm';
import { Network } from './_network.js';

@Entity({ name: 'completed_transfer' })
export class CompletedTransfer {
  constructor(props?: Partial<CompletedTransfer>) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Column('enum', { enum: Network, nullable: false, name: 'dest_network' })
  destNetwork!: Network;

  @Column('enum', { enum: Network, nullable: false, name: 'src_network' })
  srcNetwork!: Network;

  @Column('timestamp with time zone', { nullable: true })
  timestamp!: Date;

  @Column({ nullable: false, name: 'tx_hash' })
  txHash!: string;

  @Column('bigint', { nullable: false, name: 'block_number' })
  blockNumber!: bigint;
}
