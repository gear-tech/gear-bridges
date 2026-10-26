import pg from 'pg';
import type { Store } from '@subsquid/typeorm-store';
import { GearProgram } from '../model/index.js';

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

  // Configuration seeds missing roles; finalized on-chain rotations remain authoritative.
  for (const [name, id] of Object.entries(programs)) {
    await client.query(
      `INSERT INTO gear_programs (name, program_id)
       VALUES ($1, $2)
       ON CONFLICT (name) DO NOTHING`,
      [name, id],
    );
    console.log(`Program ${name} initialized`);
  }

  const ids = await client.query('SELECT name, program_id FROM gear_programs WHERE name = ANY($1::text[])', [
    Object.keys(programs),
  ]);

  await client.end();

  return new Map(ids.rows.map((row) => [row.program_id, row.name]));
}

export let programs: Map<string, ProgramName>;

export async function setPrograms() {
  programs = await init({
    [ProgramName.VftManager]: config.vftManager,
    [ProgramName.HistoricalProxy]: config.historicalProxy,
    [ProgramName.BridgingPayment]: config.bridgingPayment,
  });
}

// Reload on every callback, including retries after a failed COMMIT. A tentative
// in-memory rotation is never authority for the next transaction.
export async function reloadPrograms(store: Store) {
  const rows = await store.find(GearProgram, {});
  const roles = [ProgramName.VftManager, ProgramName.HistoricalProxy, ProgramName.BridgingPayment];
  const selected = rows.filter((row) => roles.includes(row.id as ProgramName));
  if (selected.length !== roles.length || new Set(selected.map((row) => row.programId)).size !== roles.length ||
      selected.some((row) => !/^0x[0-9a-f]{64}$/.test(row.programId))) {
    throw new Error('HOLD: incomplete or conflicting committed Gear program roles');
  }
  programs = new Map(selected.map((row) => [row.programId, row.id as ProgramName]));
}

export async function updateId(store: Store, name: ProgramName, programId: string) {
  if (!/^0x[0-9a-f]{64}$/.test(programId) || /^0x0+$/.test(programId)) throw new Error('HOLD: invalid role successor');
  const previous = [...programs].find(([, role]) => role === name)?.[0];
  if (!previous || (programs.has(programId) && programs.get(programId) !== name)) {
    throw new Error('HOLD: conflicting Gear program role rotation');
  }
  await store.save(new GearProgram({ id: name, programId }));
  programs.delete(previous);
  programs.set(programId, name);
}
