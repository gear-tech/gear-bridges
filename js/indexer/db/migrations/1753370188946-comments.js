module.exports = class Comments1753370188946 {
  name = 'Comments1753370188946';

  async up(queryRunner) {
    // Omit unnecessary tables in GraphQL schema
    await queryRunner.query(`COMMENT ON TABLE initiated_transfer IS E'@omit'`);
    await queryRunner.query(`COMMENT ON TABLE completed_transfer IS E'@omit'`);
    await queryRunner.query(`COMMENT ON TABLE vara_bridge_program IS E'@omit'`);
    await queryRunner.query(`COMMENT ON TABLE eth_bridge_program IS E'@omit'`);
    await queryRunner.query(`COMMENT ON TABLE migrations IS E'@omit'`);
  }

  async down() {}
};
