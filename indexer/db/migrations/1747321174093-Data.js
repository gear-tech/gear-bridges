module.exports = class Data1747321174093 {
    name = 'Data1747321174093'

    async up(db) {
        await db.query(`ALTER TABLE "transfer" DROP COLUMN "status"`)
        await db.query(`ALTER TABLE "transfer" ADD "status" character varying(15) NOT NULL`)
    }

    async down(db) {
        await db.query(`ALTER TABLE "transfer" ADD "status" character varying(10) NOT NULL`)
        await db.query(`ALTER TABLE "transfer" DROP COLUMN "status"`)
    }
}
