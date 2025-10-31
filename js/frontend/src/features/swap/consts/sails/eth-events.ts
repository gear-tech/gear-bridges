/* eslint-disable */

import { H256, TransactionBuilder, ActorId, QueryBuilder } from 'sails-js';
import { GearApi, BaseGearProgram, HexString } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';

export interface EthToVaraEvent {
  proof_block: BlockInclusionProof;
  proof: Array<`0x${string}`>;
  transaction_index: number | string | bigint;
  receipt_rlp: `0x${string}`;
}

export interface BlockInclusionProof {
  block: BlockGenericForBlockBody;
  headers: Array<BlockHeader>;
}

export interface BlockGenericForBlockBody {
  slot: number | string | bigint;
  proposer_index: number | string | bigint;
  parent_root: H256;
  state_root: H256;
  body: BlockBody;
}

export interface BlockBody {
  randao_reveal: H256;
  eth1_data: H256;
  graffiti: BytesFixed1;
  proposer_slashings: H256;
  attester_slashings: H256;
  attestations: H256;
  deposits: H256;
  voluntary_exits: H256;
  sync_aggregate: H256;
  execution_payload: ExecutionPayload;
  bls_to_execution_changes: H256;
  blob_kzg_commitments: H256;
}

/**
 * A homogenous collection of a fixed number of byte values.
 */
export type BytesFixed1 = [FixedArray1ForU8];

/**
 * A homogenous collection of a fixed number of values.
 *
 * NOTE: collection of length `0` is illegal.
 */
export type FixedArray1ForU8 = [Array<number>];

export interface ExecutionPayload {
  parent_hash: BytesFixed1;
  fee_recipient: BytesFixed2;
  state_root: BytesFixed1;
  receipts_root: BytesFixed1;
  logs_bloom: H256;
  prev_randao: BytesFixed1;
  block_number: number | string | bigint;
  gas_limit: number | string | bigint;
  gas_used: number | string | bigint;
  timestamp: number | string | bigint;
  extra_data: ByteList;
  base_fee_per_gas: number | string | bigint;
  block_hash: BytesFixed1;
  transactions: H256;
  withdrawals: H256;
  blob_gas_used: number | string | bigint;
  excess_blob_gas: number | string | bigint;
}

/**
 * A homogenous collection of a fixed number of byte values.
 */
export type BytesFixed2 = [FixedArray2ForU8];

/**
 * A homogenous collection of a fixed number of values.
 *
 * NOTE: collection of length `0` is illegal.
 */
export type FixedArray2ForU8 = [Array<number>];

/**
 * A homogenous collection of a variable number of byte values.
 */
export type ByteList = [ListForU8];

/**
 * A homogenous collection of a variable number of values.
 *
 * NOTE: collection of length `0` is illegal.
 */
export interface ListForU8 {
  data: `0x${string}`;
}

/**
 * According to Ethereum spec [v1.4.0](https://github.com/ethereum/consensus-specs/blob/v1.4.0/specs/phase0/beacon-chain.md#beaconblockheader).
 */
export interface BlockHeader {
  slot: number | string | bigint;
  proposer_index: number | string | bigint;
  parent_root: H256;
  state_root: H256;
  body_root: H256;
}

export interface CheckedProofs {
  receipt_rlp: `0x${string}`;
  transaction_index: number | string | bigint;
  block_number: number | string | bigint;
  slot: number | string | bigint;
}

export type Error =
  | 'DecodeReceiptEnvelopeFailure'
  | 'FailedEthTransaction'
  | 'SendFailure'
  | 'ReplyFailure'
  | 'HandleResultDecodeFailure'
  | 'MissingCheckpoint'
  | 'InvalidBlockProof'
  | 'TrieDbFailure'
  | 'InvalidReceiptProof';

export class SailsProgram {
  public readonly registry: TypeRegistry;
  public readonly ethereumEventClient: EthereumEventClient;
  private _program?: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      EthToVaraEvent: {
        proof_block: 'BlockInclusionProof',
        proof: 'Vec<Vec<u8>>',
        transaction_index: 'u64',
        receipt_rlp: 'Vec<u8>',
      },
      BlockInclusionProof: { block: 'BlockGenericForBlockBody', headers: 'Vec<BlockHeader>' },
      BlockGenericForBlockBody: {
        slot: 'u64',
        proposer_index: 'u64',
        parent_root: 'H256',
        state_root: 'H256',
        body: 'BlockBody',
      },
      BlockBody: {
        randao_reveal: 'H256',
        eth1_data: 'H256',
        graffiti: 'BytesFixed1',
        proposer_slashings: 'H256',
        attester_slashings: 'H256',
        attestations: 'H256',
        deposits: 'H256',
        voluntary_exits: 'H256',
        sync_aggregate: 'H256',
        execution_payload: 'ExecutionPayload',
        bls_to_execution_changes: 'H256',
        blob_kzg_commitments: 'H256',
      },
      BytesFixed1: '(FixedArray1ForU8)',
      FixedArray1ForU8: '([u8; 32])',
      ExecutionPayload: {
        parent_hash: 'BytesFixed1',
        fee_recipient: 'BytesFixed2',
        state_root: 'BytesFixed1',
        receipts_root: 'BytesFixed1',
        logs_bloom: 'H256',
        prev_randao: 'BytesFixed1',
        block_number: 'u64',
        gas_limit: 'u64',
        gas_used: 'u64',
        timestamp: 'u64',
        extra_data: 'ByteList',
        base_fee_per_gas: 'U256',
        block_hash: 'BytesFixed1',
        transactions: 'H256',
        withdrawals: 'H256',
        blob_gas_used: 'u64',
        excess_blob_gas: 'u64',
      },
      BytesFixed2: '(FixedArray2ForU8)',
      FixedArray2ForU8: '([u8; 20])',
      ByteList: '(ListForU8)',
      ListForU8: { data: 'Vec<u8>' },
      BlockHeader: { slot: 'u64', proposer_index: 'u64', parent_root: 'H256', state_root: 'H256', body_root: 'H256' },
      CheckedProofs: { receipt_rlp: 'Vec<u8>', transaction_index: 'u64', block_number: 'u64', slot: 'u64' },
      Error: {
        _enum: [
          'DecodeReceiptEnvelopeFailure',
          'FailedEthTransaction',
          'SendFailure',
          'ReplyFailure',
          'HandleResultDecodeFailure',
          'MissingCheckpoint',
          'InvalidBlockProof',
          'TrieDbFailure',
          'InvalidReceiptProof',
        ],
      },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
    }

    this.ethereumEventClient = new EthereumEventClient(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }

  newCtorFromCode(
    code: Uint8Array | Buffer | HexString,
    checkpoint_light_client_address: ActorId,
  ): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      null,
      'New',
      checkpoint_light_client_address,
      '[u8;32]',
      'String',
      code,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }

  newCtorFromCodeId(codeId: `0x${string}`, checkpoint_light_client_address: ActorId) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      null,
      'New',
      checkpoint_light_client_address,
      '[u8;32]',
      'String',
      codeId,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }
}

export class EthereumEventClient {
  constructor(private _program: SailsProgram) {}

  public checkProofs(message: EthToVaraEvent): TransactionBuilder<{ ok: CheckedProofs } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: CheckedProofs } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'EthereumEventClient',
      'CheckProofs',
      message,
      'EthToVaraEvent',
      'Result<CheckedProofs, Error>',
      this._program.programId,
    );
  }

  public checkpointLightClientAddress(): QueryBuilder<ActorId> {
    return new QueryBuilder<ActorId>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'EthereumEventClient',
      'CheckpointLightClientAddress',
      null,
      null,
      '[u8;32]',
    );
  }
}
