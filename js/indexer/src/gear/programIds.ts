import pg from 'pg';

import { ProgramName } from './util.js';
import { config } from './config.js';

const createClient = () =>
  new pg.Client({
    host: process.env.DB_HOST,
    port: Number(process.env.DB_PORT),
    user: process.env.DB_USER,
    password: process.env.DB_PASS,
    database: process.env.DB_NAME,
  });

export async function init(programs: Record<string, string>): Promise<Map<string, ProgramName>> {
  const client = createClient();

  await client.connect();

  const doesTableExist = await client.query(
    `SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = 'gear_programs');`,
  );

  if (!doesTableExist.rows[0].exists) {
    console.log('Creating table gear_programs...');
    await client.query(`
      CREATE TABLE gear_programs (
        name VARCHAR PRIMARY KEY,
        program_id VARCHAR(255) NOT NULL,
        created_at TIMESTAMP DEFAULT NOW()
      );
    `);
    console.log('Table gear_programs created.');
  }

  for (const [name, id] of Object.entries(programs)) {
    const res = await client.query('SELECT * FROM gear_programs WHERE name = $1', [name]);
    if (res.rows.length === 0) {
      await client.query('INSERT INTO gear_programs (name, program_id) VALUES ($1, $2)', [name, id]);
      console.log(`Inserted program ${name} with ID ${id}`);
    } else {
      console.log(`Program ${name} set to ${res.rows[0].program_id}`);
    }
  }

  const ids = await client.query('SELECT name, program_id FROM gear_programs WHERE name = ANY($1::text[])', [
    Object.keys(programs),
  ]);

  await client.end();

  return new Map(ids.rows.map((row) => [row.program_id, row.name]));
}

export async function updateId(name: string, programId: string) {
  const client = createClient();
  await client.connect();
  await client.query('UPDATE gear_programs SET program_id = $1 WHERE name = $2', [programId, name]);
  await client.end();
}

export let programs: Map<string, ProgramName>;

export async function setPrograms() {
  programs = await init({
    [ProgramName.VftManager]: config.vftManager,
    [ProgramName.HistoricalProxy]: config.historicalProxy,
    [ProgramName.BridgingPayment]: config.bridgingPayment,
    [ProgramName.CheckpointClient]: config.checkpointClient,
  });
}
