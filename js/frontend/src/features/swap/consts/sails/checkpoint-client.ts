/* eslint-disable */

import { H256, TransactionBuilder, QueryBuilder, getServiceNamePrefix, getFnNamePrefix, ZERO_ADDRESS } from 'sails-js';
import { GearApi, BaseGearProgram, HexString } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';

export interface Init {
  network: Network;
  sync_committee_current_pub_keys: FixedArrayForArrOf96U8;
  sync_committee_current_aggregate_pubkey: BytesFixed;
  sync_committee_current_branch: Array<Array<number>>;
  update: Update;
  sync_aggregate_encoded: `0x${string}`;
}

export type Network = 'Mainnet' | 'Sepolia' | 'Holesky' | 'Hoodi';

/**
 * A homogenous collection of a fixed number of values.
 *
 * NOTE: collection of length `0` is illegal.
 */
export type FixedArrayForArrOf96U8 = [Array<Array<number>>];

/**
 * A homogenous collection of a fixed number of byte values.
 */
export type BytesFixed = [FixedArrayForU8];

/**
 * A homogenous collection of a fixed number of values.
 *
 * NOTE: collection of length `0` is illegal.
 */
export type FixedArrayForU8 = [Array<number>];

export interface Update {
  signature_slot: number | string | bigint;
  attested_header: BlockHeader;
  finalized_header: BlockHeader;
  sync_committee_signature: Array<number>;
  sync_committee_next_aggregate_pubkey: BytesFixed | null;
  sync_committee_next_pub_keys: FixedArrayForArrOf96U8 | null;
  sync_committee_next_branch: Array<Array<number>> | null;
  finality_branch: Array<Array<number>>;
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

export type CheckpointError = 'OutDated' | 'NotPresent';

export type ReplayBackStatus = 'InProcess' | 'Finished';

export type ReplayBackError =
  | { AlreadyStarted: null }
  | { NotStarted: null }
  | { Verify: Error }
  | { NoFinalityUpdate: null };

export type Error =
  | { InvalidTimestamp: null }
  | { InvalidPeriod: null }
  | { LowVoteCount: null }
  | { NotActual: null }
  | { InvalidSignature: null }
  | { InvalidFinalityProof: null }
  | { InvalidNextSyncCommitteeProof: null }
  | { InvalidPublicKeys: null }
  | { InvalidSyncAggregate: null }
  | { ReplayBackRequired: { replay_back: ReplayBack | null; checkpoint: [number | string | bigint, H256] } };

/**
 * The struct contains slots of the finalized and the last checked headers.
 * This is the state of the checkpoint backfilling process.
 */
export interface ReplayBack {
  finalized_header: number | string | bigint;
  last_header: number | string | bigint;
}

export type Order = 'Direct' | 'Reverse';

export interface StateData {
  checkpoints: Array<[number | string | bigint, H256]>;
  /**
   * The field contains the data if the program is
   * replaying checkpoints back.
   */
  replay_back: ReplayBack | null;
}

export class SailsProgram {
  public readonly registry: TypeRegistry;
  public readonly serviceCheckpointFor: ServiceCheckpointFor;
  public readonly serviceReplayBack: ServiceReplayBack;
  public readonly serviceState: ServiceState;
  public readonly serviceSyncUpdate: ServiceSyncUpdate;
  private _program?: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      Init: {
        network: 'Network',
        sync_committee_current_pub_keys: 'FixedArrayForArrOf96U8',
        sync_committee_current_aggregate_pubkey: 'BytesFixed',
        sync_committee_current_branch: 'Vec<[u8; 32]>',
        update: 'Update',
        sync_aggregate_encoded: 'Vec<u8>',
      },
      Network: { _enum: ['Mainnet', 'Sepolia', 'Holesky', 'Hoodi'] },
      FixedArrayForArrOf96U8: '([[u8; 96]; 512])',
      BytesFixed: '(FixedArrayForU8)',
      FixedArrayForU8: '([u8; 48])',
      Update: {
        signature_slot: 'u64',
        attested_header: 'BlockHeader',
        finalized_header: 'BlockHeader',
        sync_committee_signature: '[u8; 192]',
        sync_committee_next_aggregate_pubkey: 'Option<BytesFixed>',
        sync_committee_next_pub_keys: 'Option<FixedArrayForArrOf96U8>',
        sync_committee_next_branch: 'Option<Vec<[u8; 32]>>',
        finality_branch: 'Vec<[u8; 32]>',
      },
      BlockHeader: { slot: 'u64', proposer_index: 'u64', parent_root: 'H256', state_root: 'H256', body_root: 'H256' },
      CheckpointError: { _enum: ['OutDated', 'NotPresent'] },
      ReplayBackStatus: { _enum: ['InProcess', 'Finished'] },
      ReplayBackError: {
        _enum: { AlreadyStarted: 'Null', NotStarted: 'Null', Verify: 'Error', NoFinalityUpdate: 'Null' },
      },
      Error: {
        _enum: {
          InvalidTimestamp: 'Null',
          InvalidPeriod: 'Null',
          LowVoteCount: 'Null',
          NotActual: 'Null',
          InvalidSignature: 'Null',
          InvalidFinalityProof: 'Null',
          InvalidNextSyncCommitteeProof: 'Null',
          InvalidPublicKeys: 'Null',
          InvalidSyncAggregate: 'Null',
          ReplayBackRequired: { replay_back: 'Option<ReplayBack>', checkpoint: '(u64, H256)' },
        },
      },
      ReplayBack: { finalized_header: 'u64', last_header: 'u64' },
      Order: { _enum: ['Direct', 'Reverse'] },
      StateData: { checkpoints: 'Vec<(u64, H256)>', replay_back: 'Option<ReplayBack>' },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
    }

    this.serviceCheckpointFor = new ServiceCheckpointFor(this);
    this.serviceReplayBack = new ServiceReplayBack(this);
    this.serviceState = new ServiceState(this);
    this.serviceSyncUpdate = new ServiceSyncUpdate(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }

  initCtorFromCode(code: Uint8Array | Buffer | HexString, init: Init): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      null,
      'Init',
      init,
      'Init',
      'String',
      code,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }

  initCtorFromCodeId(codeId: `0x${string}`, init: Init) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      null,
      'Init',
      init,
      'Init',
      'String',
      codeId,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }
}

export class ServiceCheckpointFor {
  constructor(private _program: SailsProgram) {}

  public get(
    slot: number | string | bigint,
  ): QueryBuilder<{ ok: [number | string | bigint, H256] } | { err: CheckpointError }> {
    return new QueryBuilder<{ ok: [number | string | bigint, H256] } | { err: CheckpointError }>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'ServiceCheckpointFor',
      'Get',
      slot,
      'u64',
      'Result<(u64, H256), CheckpointError>',
    );
  }
}

export class ServiceReplayBack {
  constructor(private _program: SailsProgram) {}

  public process(headers: Array<BlockHeader>): TransactionBuilder<{ ok: ReplayBackStatus } | { err: ReplayBackError }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: ReplayBackStatus } | { err: ReplayBackError }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'ServiceReplayBack',
      'Process',
      headers,
      'Vec<BlockHeader>',
      'Result<ReplayBackStatus, ReplayBackError>',
      this._program.programId,
    );
  }

  public start(
    sync_update: Update,
    sync_aggregate_encoded: `0x${string}`,
    headers: Array<BlockHeader>,
  ): TransactionBuilder<{ ok: ReplayBackStatus } | { err: ReplayBackError }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: ReplayBackStatus } | { err: ReplayBackError }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'ServiceReplayBack',
      'Start',
      [sync_update, sync_aggregate_encoded, headers],
      '(Update, Vec<u8>, Vec<BlockHeader>)',
      'Result<ReplayBackStatus, ReplayBackError>',
      this._program.programId,
    );
  }

  public subscribeToNewCheckpointEvent(
    callback: (data: { slot: number | string | bigint; tree_hash_root: H256 }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'ServiceReplayBack' && getFnNamePrefix(payload) === 'NewCheckpoint') {
        callback(
          this._program.registry
            .createType('(String, String, {"slot":"u64","tree_hash_root":"H256"})', message.payload)[2]
            .toJSON() as unknown as { slot: number | string | bigint; tree_hash_root: H256 },
        );
      }
    });
  }
}

export class ServiceState {
  constructor(private _program: SailsProgram) {}

  public get(order: Order, index_start: number, count: number): QueryBuilder<StateData> {
    return new QueryBuilder<StateData>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'ServiceState',
      'Get',
      [order, index_start, count],
      '(Order, u32, u32)',
      'StateData',
    );
  }
}

export class ServiceSyncUpdate {
  constructor(private _program: SailsProgram) {}

  public process(
    sync_update: Update,
    sync_aggregate_encoded: `0x${string}`,
  ): TransactionBuilder<{ ok: null } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: null } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'ServiceSyncUpdate',
      'Process',
      [sync_update, sync_aggregate_encoded],
      '(Update, Vec<u8>)',
      'Result<Null, Error>',
      this._program.programId,
    );
  }

  public subscribeToNewCheckpointEvent(
    callback: (data: { slot: number | string | bigint; tree_hash_root: H256 }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'ServiceSyncUpdate' && getFnNamePrefix(payload) === 'NewCheckpoint') {
        callback(
          this._program.registry
            .createType('(String, String, {"slot":"u64","tree_hash_root":"H256"})', message.payload)[2]
            .toJSON() as unknown as { slot: number | string | bigint; tree_hash_root: H256 },
        );
      }
    });
  }
}
