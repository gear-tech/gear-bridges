import { Column, Entity, PrimaryColumn } from 'typeorm';
import * as crypto from 'crypto';

@Entity({ name: 'merkle_root_in_message_queue' })
export class MerkleRootInMessageQueue {
  constructor(props?: Partial<MerkleRootInMessageQueue>) {
    Object.assign(this, props);
    this.id = crypto.randomUUID();
  }

  @PrimaryColumn()
  id!: string;

  @Column('bigint', { name: 'block_number', unique: true })
  blockNumber!: bigint;

  @Column('varchar', { name: 'merkle_root', length: 66 })
  merkleRoot!: string;
}
