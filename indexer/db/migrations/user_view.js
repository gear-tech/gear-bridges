module.exports = class UserView1753370188946 {
  name = 'UserView1753370188946';

  async up(queryRunner) {
    // Create view for postgraphile connection
    await queryRunner.query(`CREATE SCHEMA IF NOT EXISTS user_view`);
    await queryRunner.query(`CREATE VIEW user_view.pair AS SELECT * FROM public.pair`);
    await queryRunner.query(`CREATE VIEW user_view.transfer AS SELECT * FROM public.transfer`);
    await queryRunner.query(
      `CREATE VIEW user_view.gear_eth_bridge_message AS SELECT * FROM public.gear_eth_bridge_message`,
    );
  }

  async down(queryRunner) {
    await queryRunner.query(`DROP VIEW user_view.gear_eth_bridge_message`);
    await queryRunner.query(`DROP VIEW user_view.transfer`);
    await queryRunner.query(`DROP VIEW user_view.pair`);
    await queryRunner.query(`DROP SCHEMA user_view`);
  }
};
