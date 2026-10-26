import { GearApi } from '@gear-js/api';
import { TypeRegistry, Struct, u64, Bytes } from '@polkadot/types';
import { H256, QueryBuilder, getServiceNamePrefix, getFnNamePrefix, ZERO_ADDRESS } from 'sails-js';
import { StatusCb } from '../util';
import { subscribeAdvisory } from './advisory-subscription.js';

export type CheckpointError = 'OutDated' | 'NotPresent';

export type Network =
    | { Mainnet: null }
    | { Sepolia: null }
    | { Holesky: null }
    | { Hoodi: null }
    | { LocalFixture: { genesis_time: number | string | bigint; genesis_validators_root: H256 } };

export type Order = 'Direct' | 'Reverse';

export interface ReplayBack {
  finalized_header: number | string | bigint;
  last_header: number | string | bigint;
}

export interface StateData {
  checkpoints: Array<[number | string | bigint, H256]>;
  /**
   * The field contains the data if the program is
   * replaying checkpoints back.
   */
  replay_back: ReplayBack | null;
}

interface NewCheckpointEventData extends Struct {
  slot: u64;
  tree_hash_root: Bytes;
}

export class CheckpointClient {
  public readonly registry: TypeRegistry;
  public readonly serviceCheckpointFor: ServiceCheckpointFor;
  public readonly serviceSyncUpdate: ServiceSyncUpdate;
  public readonly serviceState: ServiceState;

  constructor(
    public api: GearApi,
    private readonly _programId: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      CheckpointError: { _enum: ['OutDated', 'NotPresent'] },
      ReplayBack: { finalized_header: 'u64', last_header: 'u64' },
      Order: { _enum: ['Direct', 'Reverse'] },
      Network: {"_enum":{"Mainnet":"Null","Sepolia":"Null","Holesky":"Null","Hoodi":"Null","LocalFixture":{"genesis_time":"u64","genesis_validators_root":"H256"}}},
      StateData: { checkpoints: 'Vec<(u64, H256)>', replay_back: 'Option<ReplayBack>' },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);

    this.serviceCheckpointFor = new ServiceCheckpointFor(this);
    this.serviceState = new ServiceState(this);
    this.serviceSyncUpdate = new ServiceSyncUpdate(this);
  }

  public get programId(): `0x${string}` {
    return this._programId;
  }
}

export class ServiceCheckpointFor {
  constructor(private _program: CheckpointClient) {}

  public async get(slot: number, wait = false, statusCb: StatusCb = () => {}): Promise<[number, H256]> {
    const query = new QueryBuilder<{ ok: [number, H256] } | { err: CheckpointError }>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'ServiceCheckpointFor',
      'Get',
      slot,
      'u64',
      'Result<(u64, H256), CheckpointError>',
    );

    const result = await query.call();

    if ('ok' in result) {
      return [Number(result.ok[0]), result.ok[1]];
    } else {
      if (!wait) {
        throw new Error(`Slot ${slot} is not present yet`);
      }

      let unsub: (() => void) | undefined;
      let finished = false;
      statusCb(`Slot hasn't been submitted yet`, { slot: slot.toString() });

      const [_slot, _treeHashRoot] = await new Promise<[number, H256]>((resolve, reject) => {
        statusCb(`Subscribing for new slots`);
        this._program.serviceSyncUpdate
          .subscribeToNewCheckpointEvent(async (event) => {
            statusCb(`Received new slot`, { slot: event.slot.toString() });
            if (event.slot >= slot) {
              const current = await query.call();
              if ('ok' in current) resolve([Number(current.ok[0]), current.ok[1]]);
            }
          }, reject)
          .then((_unsub) => {
            if (finished) _unsub(); else unsub = _unsub;
          })
          .catch(reject);
      }).finally(() => { finished = true; unsub?.(); });

      return [_slot, _treeHashRoot];
    }
  }
}

export class ServiceSyncUpdate {
  constructor(private _program: CheckpointClient) {}

  public subscribeToNewCheckpointEvent(
    callback: (data: { slot: number; tree_hash_root: H256 }) => void | Promise<void>,
    onError?: (error: unknown) => void | Promise<void>,
  ): Promise<() => void> {
    return subscribeAdvisory((dispatch) => this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      dispatch(() => {
        if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) return;
        const raw = message.payload.toHex();
        if (getServiceNamePrefix(raw) !== 'ServiceSyncUpdate' || getFnNamePrefix(raw) !== 'NewCheckpoint') return;
        const decoded = this._program.registry.createType('(String,String,{"slot":"u64","tree_hash_root":"H256"})', raw);
        if (decoded.toHex() !== raw.toLowerCase()) throw new Error('Malformed or trailing checkpoint event payload');
        const payload = decoded[2] as NewCheckpointEventData;
        return { slot: payload.slot.toNumber(), tree_hash_root: payload.tree_hash_root.toHex() };
      });
    }), callback, onError);
  }
}

export class ServiceState {
  constructor(private _program: CheckpointClient) {}
  public network(): QueryBuilder<Network> {
    return new QueryBuilder(this._program.api, this._program.registry, this._program.programId,
      'ServiceState', 'Network', null, null, 'Network');
  }


  public getLatestSlot(): Promise<StateData> {
    const query = new QueryBuilder<StateData>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'ServiceState',
      'Get',
      ['Reverse', 0, 1],
      '(Order, u32, u32)',
      'StateData',
    );

    return query.call();
  }
}
