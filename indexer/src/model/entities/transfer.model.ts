import { Entity, Column, PrimaryColumn, Index } from 'typeorm';
import { Network } from './_network';
import { Status } from './_status';

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

  @Index()
  @Column('timestamp with time zone', { nullable: false })
  timestamp!: Date;

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
  @Column({ nullable: false })
  destination!: string;

  @Column('enum', { enum: Status, nullable: false })
  status!: Status;

  @Index()
  @Column({ nullable: false })
  sender!: string;

  @Index()
  @Column({ nullable: false })
  receiver!: string;

  @Column('bigint', { nullable: false })
  amount!: bigint;

  @Column('bigint', { nullable: true, name: 'bridging_started_at_block' })
  bridgingStartedAtBlock?: bigint;

  @Column({ nullable: true, name: 'bridging_started_at_message_id' })
  bridgingStartedAtMessageId?: string;
}
