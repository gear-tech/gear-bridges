import { Entity, Column, PrimaryColumn } from 'typeorm';

@Entity({ name: 'gear_manager_handoff' })
export class GearManagerHandoff {
  constructor(props?: Partial<GearManagerHandoff>) { Object.assign(this, props); }

  @PrimaryColumn()
  id!: string;

  @Column('text', { unique: true })
  successor!: string;

  @Column('numeric', { name: 'minimum_receipt_slot', precision: 20, scale: 0 })
  minimumReceiptSlot!: string;

  @Column('numeric', { name: 'last_receipt_slot', precision: 20, scale: 0 })
  lastReceiptSlot!: string;

  @Column('text', { name: 'config_fingerprint' })
  configFingerprint!: string;

  @Column('bigint', { name: 'block_number' })
  blockNumber!: bigint;

  @Column('text', { name: 'block_hash' })
  blockHash!: string;

  @Column('bigint', { name: 'activation_block' })
  activationBlock!: bigint;
}
