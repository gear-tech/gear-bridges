import { BeaconBlockHeader, BeaconGenesisBlock, IBeaconBlock } from './types.js';

export interface BeaconClient {
  readonly genesisBlockTime: number;
  readonly genesisBlock: BeaconGenesisBlock;
  readonly getBlockHeader: (blockNumber: number | bigint) => Promise<BeaconBlockHeader>;
  readonly requestHeaders: (startSlot: number, endSlot: number) => Promise<BeaconBlockHeader[]>;
  readonly getBlock: (blockNumber: number | bigint) => Promise<IBeaconBlock>;
  readonly getBlockByHash: (blockHash: string) => Promise<IBeaconBlock>;
  readonly getSpec: () => Promise<Record<string, string>>;
}

class BeaconRequestError extends Error {
  constructor(public readonly status: number) {
    super('Beacon request failed with status ' + status);
  }
}

class _BeaconClient implements BeaconClient {
  private _url: string;
  private _genesisBlock: BeaconGenesisBlock;
  private _initialized: boolean;

  constructor(url: string, private readonly _deadline?: number) {
    if (!url.startsWith('https') && !url.startsWith('http')) {
      throw new Error('Invalid URL');
    }

    if (url.endsWith('/')) {
      this._url = url.slice(0, -1);
    } else {
      this._url = url;
    }
  }

  async init() {
    if (this._initialized) {
      return;
    }
    this._genesisBlock = await this._req('v1', '/beacon/genesis');
    this._initialized = true;
  }

  private async _req(
    version: 'v1' | 'v2',
    endpoint: string,
    pathParams?: string[],
    queryParams?: Record<string, string>,
    retainMetadata = false,
  ) {
    if (!endpoint.startsWith('/')) {
      endpoint = `/${endpoint}`;
    }
    let url = `${this._url}/eth/${version}${endpoint}`;
    if (pathParams && pathParams.length > 0) {
      url += `/${pathParams.join('/')}`;
    }
    if (queryParams && Object.keys(queryParams).length > 0) {
      url += `?${new URLSearchParams(queryParams).toString()}`;
    }

    const remaining = this._deadline === undefined ? undefined : this._deadline - Date.now();
    if (remaining !== undefined && (!Number.isSafeInteger(this._deadline) || remaining <= 0)) throw new Error('Original relay deadline expired');
    const response = await fetch(url, remaining === undefined ? undefined : { signal: AbortSignal.timeout(remaining) });

    if (!response.ok) {
      throw new BeaconRequestError(response.status);
    }

    const result = await response.json();

    return retainMetadata ? result : result.data;
  }

  public get genesisBlockTime(): number {
    return Number(this._genesisBlock.genesis_time);
  }

  public get genesisBlock(): BeaconGenesisBlock {
    return this._genesisBlock;
  }

  public getBlockHeader(bn: number | bigint): Promise<BeaconBlockHeader> {
    return this._req('v1', '/beacon/headers', [bn.toString()]);
  }

  public async requestHeaders(startSlot: number, endSlot: number): Promise<BeaconBlockHeader[]> {
    if (!Number.isSafeInteger(startSlot) || !Number.isSafeInteger(endSlot) || startSlot < 0 || endSlot < startSlot) {
      throw new Error('Invalid Beacon header range');
    }
    const headers: BeaconBlockHeader[] = [];
    for (let start = startSlot; start <= endSlot; start += 16) {
      const end = Math.min(start + 15, endSlot);
      const page = await Promise.all(Array.from({ length: end - start + 1 }, (_, index) =>
        this.getBlockHeader(start + index).catch((error: unknown) => {
          if (error instanceof BeaconRequestError && error.status === 404) return null;
          throw error;
        })));
      for (const header of page) if (header) headers.push(header);
    }
    return headers;
  }

  public async getBlock(bn: number | bigint | string): Promise<IBeaconBlock> {
    const result = await this._req('v2', '/beacon/blocks', [bn.toString()], undefined, true);
    if (typeof result.version !== 'string' || result.execution_optimistic !== false || result.finalized !== true) {
      throw new Error('Beacon block fork/finalized/non-optimistic metadata unavailable');
    }
    return { ...result.data.message, fork: result.version.toLowerCase() };
  }

  public getBlockByHash(blockHash: string): Promise<IBeaconBlock> {
    return this.getBlock(blockHash);
  }

  public getSpec(): Promise<Record<string, string>> {
    return this._req('v1', '/config/spec');
  }
}

export async function createBeaconClient(url: string, deadline?: number): Promise<BeaconClient> {
  const client = new _BeaconClient(url, deadline);

  await client.init();

  return client;
}
