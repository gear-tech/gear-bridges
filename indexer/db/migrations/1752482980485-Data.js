module.exports = class Data1752482980485 {
    name = 'Data1752482980485'

    async up(db) {
        await db.query(`ALTER TABLE "completed_transfer" ADD "src_network" character varying(8)`)
    }

    async down(db) {
        await db.query(`ALTER TABLE "completed_transfer" DROP COLUMN "src_network"`)
    }
}
