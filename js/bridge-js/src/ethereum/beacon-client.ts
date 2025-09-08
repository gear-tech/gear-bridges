import { BeaconBlockHeader, BeaconGenesisBlock, IBeaconBlock } from './types.js';

export interface BeaconClient {
  readonly genesisBlockTime: number;
  readonly genesisBlock: BeaconGenesisBlock;
  readonly getBlockHeader: (blockNumber: number | bigint) => Promise<BeaconBlockHeader>;
  readonly requestHeaders: (startSlot: number, endSlot: number) => Promise<BeaconBlockHeader[]>;
  readonly getBlock: (blockNumber: number | bigint) => Promise<IBeaconBlock>;
  readonly getBlockByHash: (blockHash: string) => Promise<IBeaconBlock>;
}

class _BeaconClient implements BeaconClient {
  private _url: string;
  private _genesisBlock: BeaconGenesisBlock;
  private _initialized: boolean;

  constructor(url: string) {
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

    const response = await fetch(url);

    if (!response.ok) {
      throw new Error(`Request failed with status ${response.status}`);
    }

    const result = await response.json();

    return result.data;
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
    const result = Array.from({ length: endSlot - startSlot + 1 }, (_, i) => startSlot + i);

    return (await Promise.all(result.map((slot) => this.getBlockHeader(slot).catch(() => null)))).filter(
      Boolean,
    ) as BeaconBlockHeader[];
  }

  public async getBlock(bn: number | bigint): Promise<IBeaconBlock> {
    const result = await this._req('v2', '/beacon/blocks', [bn.toString()]);

    return result.message;
  }

  public getBlockByHash(blockHash: string): Promise<IBeaconBlock> {
    return this._req('v2', '/beacon/blocks', [blockHash]);
  }
}

export async function createBeaconClient(url: string): Promise<BeaconClient> {
  const client = new _BeaconClient(url);

  await client.init();

  return client;
}
