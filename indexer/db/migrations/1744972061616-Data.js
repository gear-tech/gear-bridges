module.exports = class Data1744972061616 {
    name = 'Data1744972061616'

    async up(db) {
        await db.query(`ALTER TABLE "pair" ADD "token_supply" character varying(8) NOT NULL`)
    }

    async down(db) {
        await db.query(`ALTER TABLE "pair" DROP COLUMN "token_supply"`)
    }
}
