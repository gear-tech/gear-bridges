module.exports = class Data1733751857295 {
    name = 'Data1733751857295'

    async up(db) {
        await db.query(`ALTER TABLE "transfer" ADD "completed_at" TIMESTAMP WITH TIME ZONE`)
        await db.query(`ALTER TABLE "completed_transfer" ADD "timestamp" TIMESTAMP WITH TIME ZONE`)
    }

    async down(db) {
        await db.query(`ALTER TABLE "transfer" DROP COLUMN "completed_at"`)
        await db.query(`ALTER TABLE "completed_transfer" DROP COLUMN "timestamp"`)
    }
}
