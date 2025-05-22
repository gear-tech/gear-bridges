module.exports = class Data1747321174093 {
  name = 'Data1747321174093';

  async up(db) {
    const pendingIds = (await db.query(`SELECT id FROM "transfer" WHERE "status" = 'Pending'`)).map((row) => row.id);
    const inprogressIds = (await db.query(`SELECT id FROM "transfer" WHERE "status" = 'InProgress'`)).map(
      (row) => row.id,
    );
    await db.query(`ALTER TABLE "transfer" ALTER COLUMN "status" TYPE character varying(15)`);
    await db.query(
      `UPDATE "transfer" SET status = 'AwaitingPayment' WHERE id IN (${pendingIds.map((id) => `'${id}'`).join(',')})`,
    );
    await db.query(
      `UPDATE "transfer" SET status = 'Bridging' WHERE id IN (${inprogressIds.map((id) => `'${id}'`).join(',')})`,
    );
  }

  async down(db) {
    const pendingIds = (await db.query(`SELECT id FROM "transfer" WHERE "status" = 'AwaitingPayment'`)).map(
      (row) => row.id,
    );
    const inprogressIds = (await db.query(`SELECT id FROM "transfer" WHERE "status" = 'Bridging'`)).map(
      (row) => row.id,
    );

    await db.query(
      `UPDATE "transfer" SET status = 'Pending' WHERE id IN (${pendingIds.map((id) => `'${id}'`).join(',')})`,
    );
    await db.query(
      `UPDATE "transfer" SET status = 'InProgress' WHERE id IN (${inprogressIds.map((id) => `'${id}'`).join(',')})`,
    );

    await db.query(`ALTER TABLE "transfer" ALTER COLUMN "status" TYPE character varying(10)`);
  }
};
