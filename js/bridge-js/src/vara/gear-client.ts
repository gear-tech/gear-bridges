import { GearApi, HexString, type GearCoreMessageUserUserMessage } from '@gear-js/api';
import { hexToU8a } from '@polkadot/util';

import { VaraMessage, Proof } from './types.js';

export class GearClient {
  constructor(private _api: GearApi) {}

  /** Walk every covered block, including the original block; callers authenticate the head and application evidence. */
  public async *finalizedReplyBlocks(from: bigint, through: bigint, messageIds: ReadonlySet<string>, deadline: number,
    previousHash?: string, errors = {
      deadline: 'Original finalized reply deadline expired; reconcile before retrying',
      history: 'Finalized source reply history changed or has a gap',
    }) {
    const api = this._api;
    for (let number = from; number <= through; number++) {
      if (Date.now() >= deadline) throw new Error(errors.deadline);
      const hash = (await this._api.blocks.getBlockHash(number)).toHex();
      const header = await this._api.rpc.chain.getHeader(hash);
      if (header.number.toBigInt() !== number || (previousHash && header.parentHash.toHex() !== previousHash)) {
        throw new Error(errors.history);
      }
      const events = await this._api.blocks.getEvents(hash);
      // Keep routing after the caller's original signed-request authentication.
      yield { number, hash, events, *replies() {
        for (const { event } of events) {
          if (!api.events.gear.UserMessageSent.is(event)) continue;
          const message = event.data[0] as unknown as GearCoreMessageUserUserMessage;
          if (message.details.isNone || !messageIds.has(message.details.unwrap().to.toHex())) continue;
          yield message;
        }
      } };
      previousHash = hash;
    }
  }

  public async fetchMerkleProof(blockHash: HexString, messageHash: HexString): Promise<Proof> {
    const proof = await this._api.ethBridge.merkleProof(messageHash, blockHash);

    return {
      root: proof.root.toHex(),
      proof: proof.proof.map((item) => item.toHex()),
      numLeaves: proof.number_of_leaves.toBigInt(),
      leafIndex: proof.leaf_index.toBigInt(),
    };
  }

  public async findMessageQueuedEvent(blockHash: HexString, nonce: bigint): Promise<VaraMessage | null> {
    const events = await this._api.blocks.getEvents(blockHash);
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
