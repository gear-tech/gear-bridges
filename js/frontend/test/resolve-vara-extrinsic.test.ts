import { GearApi } from '@gear-js/api';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { loadVaraExtrinsic, resolveVaraExtrinsic } from '../src/features/history/resolve-vara-extrinsic';

vi.mock('@gear-js/api', () => ({ GearApi: vi.fn() }));

const hash = `0x${'ab'.repeat(32)}`;
const extrinsic = (value: string) => ({ hash: { toHex: () => value } });
const getBlockHash = vi.fn();
const getBlock = vi.fn();
const disconnect = vi.fn();
let ready: Promise<unknown>;
const api = () =>
  ({
    isReadyOrError: ready,
    rpc: { chain: { getBlockHash, getBlock } },
    disconnect,
  }) as unknown as GearApi;

beforeEach(() => {
  vi.resetAllMocks();
  ready = Promise.resolve();
  getBlockHash.mockImplementation((height: string) => Promise.resolve(`block-${height}`));
  getBlock.mockResolvedValue({ block: { extrinsics: [extrinsic('0xother'), extrinsic(hash)] } });
  disconnect.mockResolvedValue(undefined);
  vi.mocked(GearApi).mockImplementation(function () {
    return api();
  });
});

afterEach(() => vi.useRealTimers());

describe('recorded block coordinates', () => {
  test('disambiguates the same unsigned hash in different blocks', async () => {
    getBlock.mockImplementation((blockHash: string) =>
      Promise.resolve({
        block: { extrinsics: blockHash === 'block-123' ? [extrinsic(hash)] : [extrinsic('0xother'), extrinsic(hash)] },
      }),
    );

    await expect(resolveVaraExtrinsic(api(), '123', hash)).resolves.toBe('123-0');
    await expect(resolveVaraExtrinsic(api(), '124', hash)).resolves.toBe('124-1');
    expect(getBlock).toHaveBeenNthCalledWith(1, 'block-123');
    expect(getBlock).toHaveBeenNthCalledWith(2, 'block-124');
  });

  test('accepts hashes with different casing and preserves decimal heights', async () => {
    await expect(resolveVaraExtrinsic(api(), '37032618', hash.toUpperCase())).resolves.toBe('37032618-1');
    expect(getBlockHash).toHaveBeenCalledWith('37032618');
  });

  test('does not invent a coordinate for a hash absent from the block', async () => {
    await expect(resolveVaraExtrinsic(api(), '123', '0xmissing')).rejects.toThrow('Extrinsic not found');
  });
});

describe('archive connection lifecycle', () => {
  test('closes the connection after a successful lookup', async () => {
    await expect(loadVaraExtrinsic('ws://localhost:9944', '123', hash)).resolves.toBe('123-1');
    expect(GearApi).toHaveBeenCalledWith({ providerAddress: 'ws://localhost:9944' });
    expect(disconnect).toHaveBeenCalledOnce();
  });

  test('closes the connection when initialization fails', async () => {
    ready = Promise.reject(new Error('Wrong runtime'));
    await expect(loadVaraExtrinsic('ws://localhost:9944', '123', hash)).rejects.toThrow('Wrong runtime');
    expect(disconnect).toHaveBeenCalledOnce();
  });

  test('closes the connection when a historical RPC request fails', async () => {
    getBlock.mockRejectedValue(new Error('State already discarded'));
    await expect(loadVaraExtrinsic('ws://localhost:9944', '123', hash)).rejects.toThrow('State already discarded');
    expect(disconnect).toHaveBeenCalledOnce();
  });

  test('stops a pending connection when the query is cancelled', async () => {
    ready = new Promise(() => {});
    const controller = new AbortController();
    const lookup = loadVaraExtrinsic('ws://localhost:9944', '123', hash, controller.signal);
    controller.abort();
    await expect(lookup).rejects.toThrow();
    expect(disconnect).toHaveBeenCalledOnce();
  });

  test('does not open a connection for an already cancelled query', async () => {
    const controller = new AbortController();
    controller.abort();
    await expect(loadVaraExtrinsic('ws://localhost:9944', '123', hash, controller.signal)).rejects.toThrow();
    expect(GearApi).not.toHaveBeenCalled();
  });

  test('bounds connection attempts instead of retrying forever', async () => {
    vi.useFakeTimers();
    ready = new Promise(() => {});
    const lookup = loadVaraExtrinsic('ws://localhost:9944', '123', hash, undefined, 50);
    const rejection = expect(lookup).rejects.toThrow('Archive lookup timed out');
    await vi.advanceTimersByTimeAsync(50);
    await rejection;
    expect(disconnect).toHaveBeenCalledOnce();
  });
});
