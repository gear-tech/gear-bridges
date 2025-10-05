/**
 * @typedef {import('typeorm').MigrationInterface} MigrationInterface
 */

/**
 * @class
 * @implements {MigrationInterface}
 */
export default class AmountIsString1756821384017 {
  name = 'AmountIsString1756821384017';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "amount"`);
    await queryRunner.query(`ALTER TABLE "transfer" ADD "amount" character varying NOT NULL`);
  }

  async down(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "amount"`);
    await queryRunner.query(`ALTER TABLE "transfer" ADD "amount" bigint NOT NULL`);
  }
}
