/** @typedef {import('typeorm').MigrationInterface} MigrationInterface */
/** @implements {MigrationInterface} */
export default class RetainedManagerHandoff1792454400000 {
  name = 'RetainedManagerHandoff1792454400000';

  async up(queryRunner) {
    await queryRunner.query(`CREATE TABLE "gear_manager_handoff" ("id" character varying NOT NULL, "successor" text NOT NULL UNIQUE, "minimum_receipt_slot" numeric(20,0) NOT NULL, "last_receipt_slot" numeric(20,0) NOT NULL, "config_fingerprint" text NOT NULL, "block_number" bigint NOT NULL, "block_hash" text NOT NULL, "activation_block" bigint NOT NULL, CONSTRAINT "PK_gear_manager_handoff" PRIMARY KEY ("id"))`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DO $$ BEGIN
      IF EXISTS (SELECT 1 FROM "gear_manager_handoff")
      THEN RAISE EXCEPTION 'Cannot remove authenticated retained manager routing history';
      END IF;
    END $$`);
    await queryRunner.query(`DROP TABLE "gear_manager_handoff"`);
  }
}
