/**
 * @typedef {import('typeorm').MigrationInterface} MigrationInterface
 */

/**
 * @class
 * @implements {MigrationInterface}
 */
module.exports = class MerkleRootsAndSlots1756320525792 {
  name = 'MerkleRootsAndSlots1756320525792';

  async up(queryRunner) {
    await queryRunner.query(
      `CREATE TABLE "merkle_root_in_message_queue" ("id" character varying NOT NULL, "block_number" bigint NOT NULL, "merkle_root" character varying(66) NOT NULL, CONSTRAINT "UQ_39a86b57bcba68852cf1bfacb46" UNIQUE ("block_number"), CONSTRAINT "PK_8476d936e4d8fdffce7b9442d68" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(
      `CREATE TABLE "checkpoint_slot" ("id" character varying NOT NULL, "slot" bigint NOT NULL, "tree_hash_root" character varying(66) NOT NULL, CONSTRAINT "PK_a040790590d41d6f590f8a604e6" PRIMARY KEY ("id"))`,
    );
  }

  async down(queryRunner) {
    await queryRunner.query(`DROP TABLE "checkpoint_slot"`);
    await queryRunner.query(`DROP TABLE "merkle_root_in_message_queue"`);
  }
};
