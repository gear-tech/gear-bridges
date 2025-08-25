import { FrameSystemEventRecord } from '@polkadot/types/lookup';
import { GearApi, HexString } from '@gear-js/api';
import { compactStripLength } from '@polkadot/util';
import { Vec } from '@polkadot/types';

import { EthBridgeMessageQueuedData, VaraMessage, Proof } from './types';

export class GearClient {
  constructor(private _api: GearApi) {}

  public async getAuthoritySetIdByBlockNumber(bn: bigint): Promise<bigint> {
    const blockHash = await this._api.blocks.getBlockHash(Number(bn));
    const apiAt = await this._api.at(blockHash.toHex());

    const setId = await apiAt.query.grandpa.currentSetId();

    return setId.toBigInt();
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
    const apiAt = await this._api.at(blockHash);

    const blockEvents = (await apiAt.query.system.events()) as Vec<FrameSystemEventRecord>;

    const mqEvents = blockEvents.filter(
      ({ event }) => event.section === 'gearEthBridge' && event.method === 'MessageQueued',
    );

    const eventData = mqEvents.find(({ event }) => {
      const data = event.data as EthBridgeMessageQueuedData;
      const _nonce = data.message.nonce.toBigInt();
      return nonce === _nonce;
    })?.event.data as EthBridgeMessageQueuedData;

    if (!eventData) {
      return null;
    }

    const { message } = eventData;

    const payload = compactStripLength(message.payload.toU8a())[1];

    return {
      nonce: message.nonce.toBigInt(),
      source: message.source.toU8a(),
      destination: message.destination.toU8a(),
      payload,
    };
  }
}
