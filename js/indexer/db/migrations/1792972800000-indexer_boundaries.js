/** @typedef {import('typeorm').MigrationInterface} MigrationInterface */
/** @implements {MigrationInterface} */
export default class IndexerBoundaries1792972800000 {
  name = 'IndexerBoundaries1792972800000';

  async up(queryRunner) {
    // Preserve every existing destination and role; absent mappings are liabilities.
    await queryRunner.query(`ALTER TABLE "transfer" ALTER COLUMN "destination" DROP NOT NULL`);
    await queryRunner.query(`CREATE TABLE IF NOT EXISTS "gear_programs" ("name" varchar PRIMARY KEY, "program_id" varchar(255) NOT NULL, "created_at" timestamp DEFAULT NOW())`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DO $$ BEGIN
      IF EXISTS (SELECT 1 FROM "transfer" WHERE "destination" IS NULL)
      THEN RAISE EXCEPTION 'Cannot discard unresolved unmapped deposit liabilities';
      END IF;
    END $$`);
    await queryRunner.query(`ALTER TABLE "transfer" ALTER COLUMN "destination" SET NOT NULL`);
    // gear_programs predates this migration and may contain finalized rotations.
  }
}
