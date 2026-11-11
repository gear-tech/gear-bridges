import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { test } from 'node:test';

const configUrl = new URL('../lib/gear/config.js', import.meta.url).href;
const programId = `0x${'ab'.repeat(32)}`;
const environment = {
  GEAR_RPC_URL: 'ws://127.0.0.1:9944',
  GEAR_ARCHIVE_URL: 'http://127.0.0.1:8000',
  GEAR_FROM_BLOCK: '0',
  GEAR_VFT_MANAGER: programId,
  GEAR_HISTORICAL_PROXY: programId,
  GEAR_BRIDGING_PAYMENT: programId,
  GEAR_CHECKPOINT_CLIENT: programId,
  ETH_HTTP_RPC_URL: 'http://127.0.0.1:8545',
  GEAR_SQUID_API_KEY: 'test-only',
};

function loadConfig(env) {
  return spawnSync(
    process.execPath,
    [
      '--input-type=module',
      '-e',
      `import { config } from ${JSON.stringify(configUrl)}; console.log(JSON.stringify(config));`,
    ],
    { cwd: tmpdir(), env, encoding: 'utf8' },
  );
}

test('network parameters cannot silently fall back to another deployment', () => {
  for (const key of Object.keys(environment).filter(
    (name) => name.startsWith('GEAR_') && name !== 'GEAR_SQUID_API_KEY',
  )) {
    for (const value of [undefined, '', '   ']) {
      const result = loadConfig({ ...environment, [key]: value });
      assert.notEqual(result.status, 0, key);
      assert.match(result.stderr, new RegExp(`Missing network configuration: ${key}`));
    }
  }
});

test('invalid start blocks, URLs and program IDs fail before any connection', () => {
  const invalid = {
    GEAR_FROM_BLOCK: ['-1', '1.5', 'NaN', 'Infinity', '9007199254740992', '0x10', '1e6'],
    GEAR_RPC_URL: ['not-a-url', 'ftp://localhost'],
    GEAR_ARCHIVE_URL: ['not-a-url', 'ws://localhost'],
    GEAR_VFT_MANAGER: ['0x1234'],
    GEAR_HISTORICAL_PROXY: ['0x1234'],
    GEAR_BRIDGING_PAYMENT: ['0x1234'],
    GEAR_CHECKPOINT_CLIENT: ['0x1234'],
  };

  for (const [key, values] of Object.entries(invalid)) {
    for (const value of values) {
      const result = loadConfig({ ...environment, [key]: value });
      assert.notEqual(result.status, 0, `${key}=${value}`);
      assert.match(result.stderr, new RegExp(`Invalid network configuration: ${key}`));
    }
  }
});

test('explicit configuration accepts genesis and preserves the existing cursor schema', () => {
  const result = loadConfig(environment);
  assert.equal(result.status, 0, result.stderr);
  const config = JSON.parse(result.stdout.trim().split('\n').at(-1));
  assert.equal(config.fromBlock, 0);
  assert.equal(config.rpcUrl, environment.GEAR_RPC_URL);
  assert.equal(config.archiveUrl, environment.GEAR_ARCHIVE_URL);
  assert.equal(config.vftManager, programId);
  assert.equal(config.stateSchema, 'gear_processor');
});

test('operators can select a separate cursor schema and deployment start block', () => {
  const result = loadConfig({ ...environment, GEAR_FROM_BLOCK: '123456', GEAR_STATE_SCHEMA: 'gear_processor_dev' });
  assert.equal(result.status, 0, result.stderr);
  const config = JSON.parse(result.stdout.trim().split('\n').at(-1));
  assert.equal(config.fromBlock, 123456);
  assert.equal(config.stateSchema, 'gear_processor_dev');
});
