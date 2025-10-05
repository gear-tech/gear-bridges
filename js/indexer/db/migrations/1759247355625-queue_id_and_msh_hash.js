/**
 * @typedef {import('typeorm').MigrationInterface} MigrationInterface
 */

/**
 * @class
 * @implements {MigrationInterface}
 */
export default class QueueIdAndMshHash1759247355625 {
  name = 'QueueIdAndMshHash1759247355625';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" ADD "eth_bridge_built_in_queue_id" bigint`);
    await queryRunner.query(`ALTER TABLE "transfer" ADD "eth_bridge_built_in_msg_hash" character varying(66)`);
  }

  async down(queryRunner) {
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "eth_bridge_built_in_msg_hash"`);
    await queryRunner.query(`ALTER TABLE "transfer" DROP COLUMN "eth_bridge_built_in_queue_id"`);
  }
}
