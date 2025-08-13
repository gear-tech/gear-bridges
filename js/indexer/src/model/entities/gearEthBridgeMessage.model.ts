import { Entity, Column, PrimaryColumn, Index } from 'typeorm';

@Entity({ name: 'gear_eth_bridge_message' })
export class GearEthBridgeMessage {
  constructor(props?: Partial<GearEthBridgeMessage>) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Index({ unique: true })
  @Column({ type: 'varchar', nullable: false })
  nonce!: string;

  @Column({ type: 'bigint', nullable: false, name: 'block_number' })
  blockNumber!: bigint;
}
