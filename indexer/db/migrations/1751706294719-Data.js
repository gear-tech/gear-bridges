module.exports = class Data1751706294719 {
    name = 'Data1751706294719'

    async up(db) {
        await db.query(`CREATE TABLE "transfer" ("id" character varying NOT NULL, "tx_hash" text NOT NULL, "block_number" numeric NOT NULL, "timestamp" TIMESTAMP WITH TIME ZONE NOT NULL, "completed_at" TIMESTAMP WITH TIME ZONE, "nonce" text NOT NULL, "source_network" character varying(8) NOT NULL, "source" text NOT NULL, "dest_network" character varying(8) NOT NULL, "destination" text NOT NULL, "status" character varying(15) NOT NULL, "sender" text NOT NULL, "receiver" text NOT NULL, "amount" numeric NOT NULL, CONSTRAINT "PK_fd9ddbdd49a17afcbe014401295" PRIMARY KEY ("id"))`)
        await db.query(`CREATE INDEX "IDX_70ff8b624c3118ac3a4862d22c" ON "transfer" ("timestamp") `)
        await db.query(`CREATE INDEX "IDX_5662ca6334321160c607988dc2" ON "transfer" ("nonce") `)
        await db.query(`CREATE INDEX "IDX_1aa446c2e82f2abbb358ab5248" ON "transfer" ("source") `)
        await db.query(`CREATE INDEX "IDX_329c2ee277e5c977d4c5fbb22f" ON "transfer" ("destination") `)
        await db.query(`CREATE INDEX "IDX_9a4ceb5c3899b95c695eb5b112" ON "transfer" ("sender") `)
        await db.query(`CREATE INDEX "IDX_e95f070ab35073a24097069e6d" ON "transfer" ("receiver") `)
        await db.query(`CREATE TABLE "initiated_transfer" ("id" character varying NOT NULL, "tx_hash" text NOT NULL, "block_number" numeric NOT NULL, CONSTRAINT "PK_3f9895066e73d7868a83da3b34e" PRIMARY KEY ("id"))`)
        await db.query(`CREATE TABLE "pair" ("id" character varying NOT NULL, "vara_token" text NOT NULL, "vara_token_symbol" text NOT NULL, "vara_token_decimals" integer NOT NULL, "vara_token_name" text NOT NULL, "eth_token" text NOT NULL, "eth_token_symbol" text NOT NULL, "eth_token_decimals" integer NOT NULL, "eth_token_name" text NOT NULL, "token_supply" character varying(8) NOT NULL, "is_removed" boolean NOT NULL, "active_since_block" numeric NOT NULL, "upgraded_to" text, "active_to_block" numeric, "is_active" boolean NOT NULL, CONSTRAINT "PK_3eaf216329c5c50aedb94fa797e" PRIMARY KEY ("id"))`)
        await db.query(`CREATE INDEX "IDX_a65affac0aae345422a7eb5e5c" ON "pair" ("vara_token") `)
        await db.query(`CREATE INDEX "IDX_82dc9083e8f7bc902171fae231" ON "pair" ("eth_token") `)
        await db.query(`CREATE TABLE "completed_transfer" ("id" character varying NOT NULL, "nonce" text NOT NULL, "dest_network" character varying(8) NOT NULL, "timestamp" TIMESTAMP WITH TIME ZONE, CONSTRAINT "PK_c966d1eba60d5625faf13b457a4" PRIMARY KEY ("id"))`)
        await db.query(`CREATE UNIQUE INDEX "IDX_ab14e0c37eabeb5ba0dc3f2f78" ON "completed_transfer" ("nonce") `)
        await db.query(`CREATE TABLE "vara_bridge_program" ("id" character varying NOT NULL, "name" text NOT NULL, CONSTRAINT "PK_488fee026522a1adc6bc6c4b094" PRIMARY KEY ("id"))`)
        await db.query(`CREATE UNIQUE INDEX "IDX_0e96fff460b9d6e3e7c932aa42" ON "vara_bridge_program" ("name") `)
        await db.query(`CREATE TABLE "eth_bridge_program" ("id" character varying NOT NULL, "name" text NOT NULL, CONSTRAINT "PK_8b3eec512391cbd10a752462884" PRIMARY KEY ("id"))`)
        await db.query(`CREATE UNIQUE INDEX "IDX_4aabf86e6a8f5d0ce9c46174f7" ON "eth_bridge_program" ("name") `)
    }

    async down(db) {
        await db.query(`DROP TABLE "transfer"`)
        await db.query(`DROP INDEX "public"."IDX_70ff8b624c3118ac3a4862d22c"`)
        await db.query(`DROP INDEX "public"."IDX_5662ca6334321160c607988dc2"`)
        await db.query(`DROP INDEX "public"."IDX_1aa446c2e82f2abbb358ab5248"`)
        await db.query(`DROP INDEX "public"."IDX_329c2ee277e5c977d4c5fbb22f"`)
        await db.query(`DROP INDEX "public"."IDX_9a4ceb5c3899b95c695eb5b112"`)
        await db.query(`DROP INDEX "public"."IDX_e95f070ab35073a24097069e6d"`)
        await db.query(`DROP TABLE "initiated_transfer"`)
        await db.query(`DROP TABLE "pair"`)
        await db.query(`DROP INDEX "public"."IDX_a65affac0aae345422a7eb5e5c"`)
        await db.query(`DROP INDEX "public"."IDX_82dc9083e8f7bc902171fae231"`)
        await db.query(`DROP TABLE "completed_transfer"`)
        await db.query(`DROP INDEX "public"."IDX_ab14e0c37eabeb5ba0dc3f2f78"`)
        await db.query(`DROP TABLE "vara_bridge_program"`)
        await db.query(`DROP INDEX "public"."IDX_0e96fff460b9d6e3e7c932aa42"`)
        await db.query(`DROP TABLE "eth_bridge_program"`)
        await db.query(`DROP INDEX "public"."IDX_4aabf86e6a8f5d0ce9c46174f7"`)
    }
}
