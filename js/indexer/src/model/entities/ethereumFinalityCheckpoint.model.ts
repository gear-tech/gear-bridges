import { Entity, Column, PrimaryColumn, Index } from 'typeorm';
import type { AgreedCheckpoint } from '../../eth/finality.js';

@Entity({ name: 'ethereum_finality_checkpoint' })
export class EthereumFinalityCheckpoint {
  constructor(props?: Partial<EthereumFinalityCheckpoint>) { Object.assign(this, props); }

  @PrimaryColumn('text', { primaryKeyConstraintName: 'ethereum_finality_checkpoint_pkey' })
  id!: string;

  @Index('IDX_ethereum_finality_checkpoint_block_number')
  @Column('bigint', { name: 'block_number' })
  blockNumber!: bigint;

  @Column('jsonb')
  checkpoint!: AgreedCheckpoint;
}
