module.exports = class Init1753369468957 {
  name = 'Init1753369468957';

  async up(queryRunner) {
    await queryRunner.query(`CREATE TYPE "public"."network_enum" AS ENUM('Ethereum', 'Vara')`);
    await queryRunner.query(
      `CREATE TYPE "public"."status_enum" AS ENUM('AwaitingPayment', 'Bridging', 'Completed', 'Failed')`,
    );

    await queryRunner.query(
      `CREATE TABLE "transfer" ("id" character varying NOT NULL, "tx_hash" character varying NOT NULL, "block_number" bigint NOT NULL, "timestamp" TIMESTAMP WITH TIME ZONE NOT NULL, "completed_at" TIMESTAMP WITH TIME ZONE, "completed_at_block" bigint, "completed_at_tx_hash" character varying, "nonce" character varying NOT NULL, "source_network" "public"."network_enum" NOT NULL, "source" character varying NOT NULL, "dest_network" "public"."network_enum" NOT NULL, "destination" character varying NOT NULL, "status" "public"."status_enum" NOT NULL, "sender" character varying NOT NULL, "receiver" character varying NOT NULL, "amount" bigint NOT NULL, "bridging_started_at_block" bigint, "bridging_started_at_message_id" character varying, CONSTRAINT "PK_fd9ddbdd49a17afcbe014401295" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(`CREATE INDEX "IDX_70ff8b624c3118ac3a4862d22c" ON "transfer" ("timestamp") `);
    await queryRunner.query(`CREATE INDEX "IDX_5662ca6334321160c607988dc2" ON "transfer" ("nonce") `);
    await queryRunner.query(`CREATE INDEX "IDX_1aa446c2e82f2abbb358ab5248" ON "transfer" ("source") `);
    await queryRunner.query(`CREATE INDEX "IDX_329c2ee277e5c977d4c5fbb22f" ON "transfer" ("destination") `);
    await queryRunner.query(`CREATE INDEX "IDX_9a4ceb5c3899b95c695eb5b112" ON "transfer" ("sender") `);
    await queryRunner.query(`CREATE INDEX "IDX_e95f070ab35073a24097069e6d" ON "transfer" ("receiver") `);
    await queryRunner.query(
      `CREATE TABLE "gear_eth_bridge_message" ("id" character varying NOT NULL, "nonce" character varying NOT NULL, "block_number" bigint NOT NULL, CONSTRAINT "PK_661c2cb0e1c75454bc0a1239360" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(
      `CREATE UNIQUE INDEX "IDX_075e0c3cc416dd5e5a4c46a215" ON "gear_eth_bridge_message" ("nonce") `,
    );
    await queryRunner.query(
      `CREATE TABLE "initiated_transfer" ("id" character varying NOT NULL, "tx_hash" character varying NOT NULL, "block_number" bigint NOT NULL, CONSTRAINT "PK_3f9895066e73d7868a83da3b34e" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(
      `CREATE TABLE "pair" ("id" character varying NOT NULL, "vara_token" character varying NOT NULL, "vara_token_symbol" character varying NOT NULL, "vara_token_decimals" integer NOT NULL, "vara_token_name" character varying NOT NULL, "eth_token" character varying NOT NULL, "eth_token_symbol" character varying NOT NULL, "eth_token_decimals" integer NOT NULL, "eth_token_name" character varying NOT NULL, "token_supply" "public"."network_enum" NOT NULL, "is_removed" boolean NOT NULL, "active_since_block" bigint NOT NULL, "upgraded_to" character varying, "active_to_block" bigint, "is_active" boolean NOT NULL, CONSTRAINT "PK_3eaf216329c5c50aedb94fa797e" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(`CREATE INDEX "IDX_a65affac0aae345422a7eb5e5c" ON "pair" ("vara_token") `);
    await queryRunner.query(`CREATE INDEX "IDX_82dc9083e8f7bc902171fae231" ON "pair" ("eth_token") `);
    await queryRunner.query(
      `CREATE TABLE "completed_transfer" ("id" character varying NOT NULL, "dest_network" "public"."network_enum" NOT NULL, "src_network" "public"."network_enum" NOT NULL, "timestamp" TIMESTAMP WITH TIME ZONE, "tx_hash" character varying NOT NULL, "block_number" bigint NOT NULL, CONSTRAINT "PK_c966d1eba60d5625faf13b457a4" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(
      `CREATE TABLE "vara_bridge_program" ("id" character varying NOT NULL, "name" character varying NOT NULL, CONSTRAINT "PK_488fee026522a1adc6bc6c4b094" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(`CREATE UNIQUE INDEX "IDX_0e96fff460b9d6e3e7c932aa42" ON "vara_bridge_program" ("name") `);
    await queryRunner.query(
      `CREATE TABLE "eth_bridge_program" ("id" character varying NOT NULL, "name" character varying NOT NULL, CONSTRAINT "PK_8b3eec512391cbd10a752462884" PRIMARY KEY ("id"))`,
    );
    await queryRunner.query(`CREATE UNIQUE INDEX "IDX_4aabf86e6a8f5d0ce9c46174f7" ON "eth_bridge_program" ("name") `);
  }

  async down(queryRunner) {
    await queryRunner.query(`DROP INDEX "public"."IDX_4aabf86e6a8f5d0ce9c46174f7"`);
    await queryRunner.query(`DROP TABLE "eth_bridge_program"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_0e96fff460b9d6e3e7c932aa42"`);
    await queryRunner.query(`DROP TABLE "vara_bridge_program"`);
    await queryRunner.query(`DROP TABLE "completed_transfer"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_82dc9083e8f7bc902171fae231"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_a65affac0aae345422a7eb5e5c"`);
    await queryRunner.query(`DROP TABLE "pair"`);
    await queryRunner.query(`DROP TABLE "initiated_transfer"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_075e0c3cc416dd5e5a4c46a215"`);
    await queryRunner.query(`DROP TABLE "gear_eth_bridge_message"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_e95f070ab35073a24097069e6d"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_9a4ceb5c3899b95c695eb5b112"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_329c2ee277e5c977d4c5fbb22f"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_1aa446c2e82f2abbb358ab5248"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_5662ca6334321160c607988dc2"`);
    await queryRunner.query(`DROP INDEX "public"."IDX_70ff8b624c3118ac3a4862d22c"`);
    await queryRunner.query(`DROP TABLE "transfer"`);
    await queryRunner.query(`DROP TYPE "public"."network_enum"`);
    await queryRunner.query(`DROP TYPE "public"."status_enum"`);
  }
};
