/**
 * @typedef {import('typeorm').MigrationInterface} MigrationInterface
 */

/**
 * @class
 * @implements {MigrationInterface}
 */
export default class SaveMerkleRootSubmissionData1760537236183 {
  name = 'SaveMerkleRootSubmissionData1760537236183';

  async up(queryRunner) {
    await queryRunner.query(`ALTER TABLE "merkle_root_in_message_queue" ADD "submitted_at_block" bigint`);
    await queryRunner.query(
      `ALTER TABLE "merkle_root_in_message_queue" ADD "submitted_at_tx_hash" character varying(66)`,
    );
    await queryRunner.query(
      `ALTER TABLE "merkle_root_in_message_queue" DROP CONSTRAINT "UQ_39a86b57bcba68852cf1bfacb46"`,
    );
  }

  async down(queryRunner) {
    await queryRunner.query(
      `ALTER TABLE "merkle_root_in_message_queue" ADD CONSTRAINT "UQ_39a86b57bcba68852cf1bfacb46" UNIQUE ("block_number")`,
    );
    await queryRunner.query(`ALTER TABLE "merkle_root_in_message_queue" DROP COLUMN "submitted_at_tx_hash"`);
    await queryRunner.query(`ALTER TABLE "merkle_root_in_message_queue" DROP COLUMN "submitted_at_block"`);
  }
}
