/** @typedef {import('typeorm').MigrationInterface} MigrationInterface */
/** @implements {MigrationInterface} */
export default class NativeEthereumEvidence1792627200000 {
  name = 'NativeEthereumEvidence1792627200000';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" ADD "legacy_ethereum_auth" jsonb`);
    await queryRunner.query(`ALTER TABLE "completed_transfer" ADD "legacy_receipt_auth" jsonb`);

    // Archive both bindings and the exact pre-cutover SQL projection before clearing old authority.
    // Existing legacy_resolution and checkpoint/history rows remain immutable.
    await queryRunner.query(`UPDATE "transfer" SET
      "legacy_ethereum_auth" = jsonb_build_object(
        'sourceReceiptAuth', "source_receipt_auth", 'completionAuth', "completion_auth",
        'status', "status"::text, 'completedAt', "completed_at"::text,
        'completedAtBlock', "completed_at_block"::text, 'completedAtTxHash', "completed_at_tx_hash")
      WHERE ("source_receipt_auth" IS NOT NULL AND "source_receipt_auth"->>'version' IS DISTINCT FROM '2')
         OR ("completion_auth" IS NOT NULL AND "completion_auth"->>'version' IS DISTINCT FROM '2')`);
    await queryRunner.query(`UPDATE "receipt_settlement" SET
      "legacy_matched" = COALESCE("legacy_matched", "matched"), "matched" = false
      WHERE "matched" AND "id" IN (SELECT "completion_auth"->>'receiptSettlementId' FROM "transfer"
        WHERE "completion_auth"->>'kind' = 'gear-settlement'
          AND "completion_auth"->>'version' IS DISTINCT FROM '2')`);
    await queryRunner.query(`UPDATE "transfer" SET
      "status" = 'Bridging', "completed_at" = NULL, "completed_at_block" = NULL, "completed_at_tx_hash" = NULL
      WHERE "legacy_ethereum_auth" IS NOT NULL AND (
        ("source_network" = 'Vara' AND "dest_network" = 'Ethereum' AND "status"::text = 'Completed'
          AND "completion_auth" IS NOT NULL AND "completion_auth"->>'version' IS DISTINCT FROM '2') OR
        ("source_network" = 'Ethereum' AND "dest_network" = 'Vara' AND "status"::text IN ('Completed', 'Refunded') AND (
          ("source_receipt_auth" IS NOT NULL AND "source_receipt_auth"->>'version' IS DISTINCT FROM '2') OR
          ("completion_auth" IS NOT NULL AND "completion_auth"->>'version' IS DISTINCT FROM '2'))))`);
    await queryRunner.query(`UPDATE "transfer" SET
      "source_receipt_auth" = CASE WHEN "source_receipt_auth"->>'version' = '2' THEN "source_receipt_auth" ELSE NULL END,
      "completion_auth" = CASE WHEN "completion_auth"->>'version' = '2' THEN "completion_auth" ELSE NULL END
      WHERE "legacy_ethereum_auth" IS NOT NULL`);
    await queryRunner.query(`UPDATE "completed_transfer" SET
      "legacy_receipt_auth" = jsonb_build_object('receiptAuth', "receipt_auth", 'matched', "matched",
        'timestamp', "timestamp"::text, 'blockNumber', "block_number"::text, 'txHash', "tx_hash"),
      "legacy_matched" = COALESCE("legacy_matched", "matched"), "matched" = false, "receipt_auth" = NULL
      WHERE "receipt_auth" IS NOT NULL AND "receipt_auth"->>'version' IS DISTINCT FROM '2'`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DO $$ BEGIN
      IF EXISTS (SELECT 1 FROM "transfer" WHERE "legacy_ethereum_auth" IS NOT NULL
        OR "source_receipt_auth" IS NOT NULL OR "completion_auth" IS NOT NULL)
        OR EXISTS (SELECT 1 FROM "completed_transfer" WHERE "legacy_receipt_auth" IS NOT NULL OR "receipt_auth" IS NOT NULL)
        OR EXISTS (SELECT 1 FROM "ethereum_finality_checkpoint" WHERE "checkpoint"->>'schemaVersion' = '2')
      THEN RAISE EXCEPTION 'Cannot remove preserved Ethereum authorization archives or native evidence';
      END IF;
    END $$`);
    await queryRunner.query(`ALTER TABLE "completed_transfer" DROP COLUMN "legacy_receipt_auth"`);
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "legacy_ethereum_auth"`);
  }
}
