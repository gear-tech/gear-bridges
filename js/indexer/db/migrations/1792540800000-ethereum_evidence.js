/** @typedef {import('typeorm').MigrationInterface} MigrationInterface */
/** @implements {MigrationInterface} */
export default class EthereumEvidence1792540800000 {
  name = 'EthereumEvidence1792540800000';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer"
      ADD "source_receipt_auth" jsonb,
      ADD "completion_auth" jsonb,
      ADD "legacy_resolution" jsonb`);
    await queryRunner.query(`ALTER TABLE "completed_transfer" ADD "receipt_auth" jsonb, ADD "legacy_matched" boolean`);
    await queryRunner.query(`ALTER TABLE "receipt_settlement" ADD "legacy_matched" boolean`);
    await queryRunner.query(`CREATE TABLE "ethereum_finality_checkpoint" (
      "id" text NOT NULL, "block_number" bigint NOT NULL, "checkpoint" jsonb NOT NULL,
      CONSTRAINT "ethereum_finality_checkpoint_pkey" PRIMARY KEY ("id"))`);
    await queryRunner.query(`CREATE INDEX "IDX_ethereum_finality_checkpoint_block_number"
      ON "ethereum_finality_checkpoint" ("block_number")`);

    // Keep the exact SQL timestamp precision and bigint digits, not JS Date/Number projections.
    // Preservation and invalidation share the migration transaction; NULL never authorizes a terminal state.
    await queryRunner.query(`UPDATE "transfer" SET
      "legacy_resolution" = COALESCE("legacy_resolution", jsonb_build_object(
        'status', "status"::text,
        'completedAt', "completed_at"::text,
        'completedAtBlock', "completed_at_block"::text,
        'completedAtTxHash', "completed_at_tx_hash")),
      "status" = 'Bridging', "completed_at" = NULL, "completed_at_block" = NULL, "completed_at_tx_hash" = NULL
      WHERE "completion_auth" IS NULL AND (
        ("source_network" = 'Vara' AND "dest_network" = 'Ethereum' AND "status"::text = 'Completed') OR
        ("source_network" = 'Ethereum' AND "dest_network" = 'Vara' AND "status"::text IN ('Completed', 'Refunded')))`);
    await queryRunner.query(`UPDATE "completed_transfer" SET
      "legacy_matched" = COALESCE("legacy_matched", "matched"), "matched" = false
      WHERE "src_network" = 'Vara' AND "dest_network" = 'Ethereum' AND "receipt_auth" IS NULL`);
    await queryRunner.query(`UPDATE "receipt_settlement" SET
      "legacy_matched" = COALESCE("legacy_matched", "matched"), "matched" = false WHERE "matched"`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DO $$ BEGIN
      IF EXISTS (SELECT 1 FROM "transfer" WHERE "legacy_resolution" IS NOT NULL
        OR "source_receipt_auth" IS NOT NULL OR "completion_auth" IS NOT NULL)
        OR EXISTS (SELECT 1 FROM "completed_transfer" WHERE "receipt_auth" IS NOT NULL OR "legacy_matched" IS NOT NULL)
        OR EXISTS (SELECT 1 FROM "receipt_settlement" WHERE "legacy_matched" IS NOT NULL)
        OR EXISTS (SELECT 1 FROM "ethereum_finality_checkpoint")
      THEN RAISE EXCEPTION 'Cannot remove preserved Ethereum projections or authenticated receipt/finality evidence';
      END IF;
    END $$`);
    await queryRunner.query(`DROP TABLE "ethereum_finality_checkpoint"`);
    await queryRunner.query(`ALTER TABLE "receipt_settlement" DROP COLUMN "legacy_matched"`);
    await queryRunner.query(`ALTER TABLE "completed_transfer" DROP COLUMN "legacy_matched", DROP COLUMN "receipt_auth"`);
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "legacy_resolution", DROP COLUMN "completion_auth", DROP COLUMN "source_receipt_auth"`);
  }
}
