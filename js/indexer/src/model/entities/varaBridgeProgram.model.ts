import { Entity, Column, PrimaryColumn, Index } from 'typeorm';

@Entity({ name: 'vara_bridge_program' })
export class VaraBridgeProgram {
  constructor(props?: Partial<VaraBridgeProgram>) {
    Object.assign(this, props);
  }

  @PrimaryColumn()
  id!: string;

  @Index({ unique: true })
  @Column({ nullable: false })
  name!: string;
}
