import { Struct, Vec, GenericEventData, U256, u8 } from '@polkadot/types';
import { H256 } from '@polkadot/types/interfaces';
import { HexString } from '@gear-js/api';

export interface EthBridgeMessage extends Struct {
  readonly nonce: U256;
  readonly source: H256;
  readonly destination: H256;
  readonly payload: Vec<u8>;
}

export interface EthBridgeMessageQueuedData extends GenericEventData {
  readonly message: EthBridgeMessage;
  readonly hash_: H256;
}

export interface VaraMessage {
  readonly nonce: bigint;
  readonly source: Uint8Array;
  readonly destination: Uint8Array;
  readonly payload: Uint8Array;
}

export interface Proof {
  root: HexString;
  proof: HexString[];
  numLeaves: bigint;
  leafIndex: bigint;
}
