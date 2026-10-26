import { Entity, Column, PrimaryColumn } from 'typeorm';

// Maps the existing startup-seeded role table; never replaces retained roles.
@Entity({ name: 'gear_programs' })
export class GearProgram {
  constructor(props?: Partial<GearProgram>) { Object.assign(this, props); }

  @PrimaryColumn({ name: 'name' })
  id!: string;

  @Column({ name: 'program_id' })
  programId!: string;
}
