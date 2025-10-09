/**
 * @typedef {import('typeorm').MigrationInterface} MigrationInterface
 */

/**
 * @class
 * @implements {MigrationInterface}
 */
export default class IsPriorityFeePaid1760014621133 {
  name = 'IsPriorityFeePaid1760014621133';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" ADD "is_priority_fee_paid" boolean NOT NULL DEFAULT false`);
  }

  async down(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "is_priority_fee_paid"`);
  }
}
