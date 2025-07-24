import { Entity, Column, PrimaryColumn, Index } from 'typeorm';

@Entity({ name: 'eth_bridge_program' })
export class EthBridgeProgram {
  constructor(props?: Partial<EthBridgeProgram>) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Index({ unique: true })
  @Column({ nullable: false })
  name!: string;
}
