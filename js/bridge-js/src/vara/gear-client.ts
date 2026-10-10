import { GearApi, HexString } from '@gear-js/api';
import { hexToU8a } from '@polkadot/util';

import { VaraMessage, Proof } from './types.js';

export class GearClient {
  constructor(private _api: GearApi) {}

  public async fetchMerkleProof(blockNumber: number, messageHash: HexString): Promise<Proof> {
    const blockHash = await this._api.blocks.getBlockHash(blockNumber);
    const proof = await this._api.ethBridge.merkleProof(messageHash, blockHash);

    return {
      root: proof.root.toHex(),
      proof: proof.proof.map((item) => item.toHex()),
      numLeaves: proof.number_of_leaves.toBigInt(),
      leafIndex: proof.leaf_index.toBigInt(),
    };
  }

  public async findMessageQueuedEvent(blockNumber: number, nonce: bigint): Promise<VaraMessage | null> {
    const blockHash = await this._api.blocks.getBlockHash(blockNumber);
    const events = await this._api.blocks.getEvents(blockHash.toHex());
    const messages = events
      .filter(({ event }) => event.section === 'gearEthBridge' && event.method === 'MessageQueued')
      .map(({ event }) => event.data[0] as unknown as import('./types.js').EthBridgeMessage)
      .filter((message) => message.nonce.toBigInt() === nonce);
    if (messages.length === 0) return null;
    if (messages.length !== 1) throw new Error('Ambiguous source MessageQueued evidence');
    const [message] = messages;
    return {
      nonce: message.nonce.toBigInt(),
      source: hexToU8a(message.source.toHex()),
      destination: hexToU8a(message.destination.toHex()),
      payload: message.payload.toU8a(true),
    };
  }
}
