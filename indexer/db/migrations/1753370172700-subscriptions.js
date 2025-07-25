module.exports = class Subscriptions1753370172700 {
  name = 'Subscriptions1753370172700';

  async up(queryRunner) {
    // Create triggers for postgraphile subscriptions
    await queryRunner.query(`CREATE OR REPLACE FUNCTION notify_transfers_count() RETURNS trigger AS $$
        DECLARE
          count INTEGER;
        BEGIN
          SELECT COUNT(*) INTO count FROM public.transfer;
          PERFORM pg_notify('transfers_changed', count::text);
          RETURN NULL;
        END;
        $$ LANGUAGE plpgsql;`);

    await queryRunner.query(`CREATE TRIGGER transfers_notify_trigger
        AFTER INSERT ON transfer
        FOR EACH STATEMENT
        EXECUTE FUNCTION notify_transfers_count();`);
  }

  async down(queryRunner) {
    await queryRunner.query(`DROP TRIGGER transfers_notify_trigger ON transfer`);
    await queryRunner.query(`DROP FUNCTION notify_transfers_count()`);
  }
};
