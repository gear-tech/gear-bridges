import { Entity, Column, PrimaryColumn, Index } from 'typeorm';
import { Network } from './_network';

@Entity({ name: 'pair' })
export class Pair {
  constructor(props?: Partial<Pair>) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Index()
  @Column({ nullable: false, name: 'vara_token' })
  varaToken!: string;

  @Column({ nullable: false, name: 'vara_token_symbol' })
  varaTokenSymbol!: string;

  @Column({ nullable: false, name: 'vara_token_decimals' })
  varaTokenDecimals!: number;

  @Column({ nullable: false, name: 'vara_token_name' })
  varaTokenName!: string;

  @Index()
  @Column({ nullable: false, name: 'eth_token' })
  ethToken!: string;

  @Column({ nullable: false, name: 'eth_token_symbol' })
  ethTokenSymbol!: string;

  @Column({ nullable: false, name: 'eth_token_decimals' })
  ethTokenDecimals!: number;

  @Column({ nullable: false, name: 'eth_token_name' })
  ethTokenName!: string;

  @Column('enum', { enum: Network, nullable: false, name: 'token_supply' })
  tokenSupply!: Network;

  @Column({ nullable: false, name: 'is_removed' })
  isRemoved!: boolean;

  @Column('bigint', { nullable: false, name: 'active_since_block' })
  activeSinceBlock!: bigint;

  @Column({ nullable: true, name: 'upgraded_to' })
  upgradedTo?: string;

  @Column('bigint', { nullable: true, name: 'active_to_block' })
  activeToBlock?: bigint;

  @Column({ nullable: false, name: 'is_active' })
  isActive!: boolean;
}
