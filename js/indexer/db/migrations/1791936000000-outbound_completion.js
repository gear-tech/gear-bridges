/** @typedef {import('typeorm').MigrationInterface} MigrationInterface */
/** @implements {MigrationInterface} */
export default class OutboundCompletion1791936000000 {
  name = 'OutboundCompletion1791936000000';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "completed_transfer" ADD "outbound_receipt" jsonb`);
    await queryRunner.query(`ALTER TABLE "completed_transfer" ADD "matched" boolean NOT NULL DEFAULT false`);
    await queryRunner.query(`CREATE INDEX "IDX_completed_transfer_matched" ON "completed_transfer" ("matched")`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DROP INDEX "IDX_completed_transfer_matched"`);
    await queryRunner.query(`ALTER TABLE "completed_transfer" DROP COLUMN "matched"`);
    await queryRunner.query(`ALTER TABLE "completed_transfer" DROP COLUMN "outbound_receipt"`);
  }
}
