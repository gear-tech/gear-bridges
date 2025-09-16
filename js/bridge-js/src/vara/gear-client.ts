import { GearApi, HexString } from '@gear-js/api';
import { hexToU8a } from '@polkadot/util';

import { VaraMessage, Proof } from './types.js';

export class GearClient {
  constructor(private _api: GearApi) {}

  public async getAuthoritySetIdByBlockNumber(bn: bigint): Promise<bigint> {
    const blockHash = await this._api.blocks.getBlockHash(Number(bn));
    const apiAt = await this._api.at(blockHash.toHex());

    const setId = await apiAt.query.grandpa.currentSetId();

    return setId.toBigInt();
  }

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
    const msg = await this._api.ethBridge.events.findGearEthBridgeMessageByNonce({ nonce, fromBlock: blockNumber });

    if (!msg) {
      return null;
    }

    return {
      nonce: msg.nonce,
      source: hexToU8a(msg.source),
      destination: hexToU8a(msg.destination),
      payload: hexToU8a(msg.payload),
    };
  }
}
