import { Column, Entity, PrimaryColumn } from 'typeorm';
import * as crypto from 'crypto';

@Entity({ name: 'checkpoint_slot' })
export class CheckpointSlot {
  constructor(props?: Partial<CheckpointSlot>) {
    Object.assign(this, props);
    this.id = crypto.randomUUID();
  }

  @PrimaryColumn()
  id!: string;

  @Column('bigint')
  slot!: bigint;

  @Column('varchar', { length: 66, name: 'tree_hash_root' })
  treeHashRoot!: string;
}
