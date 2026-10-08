/** @typedef {import('typeorm').MigrationInterface} MigrationInterface */
/** @implements {MigrationInterface} */
export default class ReceiptIdentity1791244800000 {
  name = 'ReceiptIdentity1791244800000';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" ADD "source_transaction_index" bigint`);
    await queryRunner.query(`ALTER TABLE "transfer" ADD "source_log_index" bigint`);
    await queryRunner.query(`ALTER TABLE "transfer" ADD "receipt_slot" bigint`);
    await queryRunner.query(`CREATE INDEX "IDX_transfer_receipt_slot" ON "transfer" ("receipt_slot")`);
    await queryRunner.query(`CREATE TABLE "receipt_settlement" ("id" character varying NOT NULL, "deposit" jsonb NOT NULL, "timestamp" timestamp with time zone NOT NULL, "block_number" bigint NOT NULL, "tx_hash" character varying NOT NULL, "matched" boolean NOT NULL DEFAULT false, CONSTRAINT "PK_receipt_settlement" PRIMARY KEY ("id"))`);
    await queryRunner.query(`CREATE INDEX "IDX_receipt_settlement_matched" ON "receipt_settlement" ("matched")`);
    await queryRunner.query(`CREATE TABLE "receipt_relay" ("id" character varying NOT NULL, "block_number" bigint NOT NULL, CONSTRAINT "PK_receipt_relay" PRIMARY KEY ("id"))`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DROP TABLE "receipt_relay"`);
    await queryRunner.query(`DROP TABLE "receipt_settlement"`);
    await queryRunner.query(`DROP INDEX "IDX_transfer_receipt_slot"`);
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "receipt_slot"`);
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "source_log_index"`);
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "source_transaction_index"`);
  }
}
