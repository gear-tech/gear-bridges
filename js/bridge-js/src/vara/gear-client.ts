import { GearApi, HexString } from '@gear-js/api';
import { hexToU8a } from '@polkadot/util';

import { VaraMessage, Proof } from './types.js';

export class GearClient {
  constructor(private _api: GearApi) {}

  public async getAuthoritySetIdByBlockNumber(bn: bigint): Promise<bigint> {
    const [blockHash, prevBlockHash] = await Promise.all([
      this._api.blocks.getBlockHash(Number(bn)),
      this._api.blocks.getBlockHash(Number(bn) - 1),
    ]);

    const [apiAt, prevApiAt] = await Promise.all([
      this._api.at(blockHash.toHex()),
      this._api.at(prevBlockHash.toHex()),
    ]);
    const [setId, prevSetId] = await Promise.all([
      apiAt.query.grandpa.currentSetId(),
      prevApiAt.query.grandpa.currentSetId(),
    ]);

    if (prevSetId !== setId) {
      return prevSetId.toBigInt();
    } else {
      return setId.toBigInt();
    }
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
