import { Column, Entity, PrimaryColumn } from 'typeorm';

@Entity({ name: 'merkle_root_in_message_queue' })
export class MerkleRootInMessageQueue {
  constructor(props?: MerkleRootInMessageQueue) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Column('bigint', { name: 'block_number' })
  blockNumber!: bigint;

  @Column('varchar', { name: 'merkle_root', length: 66 })
  merkleRoot!: string;

  @Column('bigint', { name: 'submitted_at_block', nullable: true })
  submittedAtBlock: bigint;

  @Column('varchar', { name: 'submitted_at_tx_hash', length: 66, nullable: true })
  submittedAtTxHash!: string;
}
