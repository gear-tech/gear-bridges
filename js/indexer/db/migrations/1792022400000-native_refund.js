/** @typedef {import('typeorm').MigrationInterface} MigrationInterface */
/** @implements {MigrationInterface} */
export default class NativeRefund1792022400000 {
  name = 'NativeRefund1792022400000';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TYPE "public"."status_enum" ADD VALUE 'Refunded'`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DO $$ BEGIN
      IF EXISTS (SELECT 1 FROM "transfer" WHERE "status"::text = 'Refunded')
        OR EXISTS (SELECT 1 FROM "receipt_settlement" WHERE "deposit"->>'outcome' = 'Refunded')
      THEN RAISE EXCEPTION 'Cannot remove Refunded status while authenticated refunds exist';
      END IF;
    END $$`);
    await queryRunner.query(`ALTER TYPE "public"."status_enum" RENAME TO "status_enum_with_refund"`);
    await queryRunner.query(`CREATE TYPE "public"."status_enum" AS ENUM('AwaitingPayment', 'Bridging', 'Completed', 'Failed')`);
    await queryRunner.query(`ALTER TABLE "transfer" ALTER COLUMN "status" TYPE "public"."status_enum" USING "status"::text::"public"."status_enum"`);
    await queryRunner.query(`DROP TYPE "public"."status_enum_with_refund"`);
  }
}
