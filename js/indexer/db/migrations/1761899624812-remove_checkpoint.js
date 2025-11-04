/**
 * @typedef {import('typeorm').MigrationInterface} MigrationInterface
 */

/**
 * @class
 * @implements {MigrationInterface}
 */
export default class RemoveCheckpoint1761899624812 {
  name = 'RemoveCheckpoint1761899624812';

  async up(queryRunner) {
    await queryRunner.query(`DROP TABLE checkpoint_slot`);
  }

  async down(queryRunner) {
    await queryRunner.query(
      `CREATE TABLE "checkpoint_slot" ("id" character varying NOT NULL, "slot" bigint NOT NULL, "tree_hash_root" character varying(66) NOT NULL, CONSTRAINT "PK_a040790590d41d6f590f8a604e6" PRIMARY KEY ("id"))`,
    );
  }
}
