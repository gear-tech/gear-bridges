/**
 * @typedef {import('typeorm').MigrationInterface} MigrationInterface
 */

/**
 * @class
 * @implements {MigrationInterface}
 */
export default class MerkleRootMaxBlockNumberFieild1758799400448 {
  name = 'MerkleRootMaxBlockNumberFieild1758799400448';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "merkle_root_in_message_queue" ADD "max_block_number" bigint NOT NULL`);
  }

  async down(queryRunner) {
    await queryRunner.query(`ALTER TABLE "merkle_root_in_message_queue" DROP COLUMN "max_block_number"`);
  }
}
