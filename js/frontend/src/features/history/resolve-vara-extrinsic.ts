import { GearApi } from '@gear-js/api';

// Unsigned gear.run hashes repeat across blocks. Resolve within the recorded block.
async function resolveVaraExtrinsic(api: GearApi, blockNumber: string, txHash: string) {
  const blockHash = await api.rpc.chain.getBlockHash(blockNumber);
  const { block } = await api.rpc.chain.getBlock(blockHash);
  const index = block.extrinsics.findIndex(
    (extrinsic) => extrinsic.hash.toHex().toLowerCase() === txHash.toLowerCase(),
  );

  if (index === -1) throw new Error('Extrinsic not found in the recorded bridge block');

  return `${blockNumber}-${index}`;
}

async function loadVaraExtrinsic(
  archive: string,
  blockNumber: string,
  txHash: string,
  signal?: AbortSignal,
  timeoutMs = 15_000,
) {
  signal?.throwIfAborted();

  // Keep a reference while connecting so a failed/aborted connection can also be closed.
  const api = new GearApi({ providerAddress: archive });
  let timer: ReturnType<typeof setTimeout> | undefined;
  let abort = () => {};

  try {
    const interrupted = new Promise<never>((_resolve, reject) => {
      timer = setTimeout(() => reject(new Error('Archive lookup timed out')), timeoutMs);
      abort = () => reject(new DOMException('Archive lookup cancelled', 'AbortError'));
      signal?.addEventListener('abort', abort, { once: true });
    });
    const lookup = async () => {
      await api.isReadyOrError;

      return resolveVaraExtrinsic(api, blockNumber, txHash);
    };

    return await Promise.race([lookup(), interrupted]);
  } finally {
    clearTimeout(timer);
    signal?.removeEventListener('abort', abort);
    await api.disconnect();
  }
}

export { loadVaraExtrinsic, resolveVaraExtrinsic };
