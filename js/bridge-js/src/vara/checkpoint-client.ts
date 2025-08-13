import { GearApi, BaseGearProgram, decodeAddress } from '@gear-js/api';
import { TypeRegistry, Struct, u64, Bytes } from '@polkadot/types';
import { H256, throwOnErrorReply, getServiceNamePrefix, getFnNamePrefix, ZERO_ADDRESS } from 'sails-js';
import { logger } from '../util';

export type CheckpointError = 'OutDated' | 'NotPresent';

interface NewCheckpointEventData extends Struct {
  slot: u64;
  tree_hash_root: Bytes;
}

export class CheckpointClient {
  public readonly registry: TypeRegistry;
  public readonly serviceCheckpointFor: ServiceCheckpointFor;
  public readonly serviceSyncUpdate: ServiceSyncUpdate;
  private _program: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      CheckpointError: { _enum: ['OutDated', 'NotPresent'] },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
    }

    this.serviceCheckpointFor = new ServiceCheckpointFor(this);
    this.serviceSyncUpdate = new ServiceSyncUpdate(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }
}

export class ServiceCheckpointFor {
  constructor(private _program: CheckpointClient) {}

  public async get(slot: number): Promise<[number, H256]> {
    const payload = this._program.registry
      .createType('(String, String, u64)', ['ServiceCheckpointFor', 'Get', slot])
      .toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: ZERO_ADDRESS,
      payload,
      value: 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const [_service, _method, result] = this._program.registry.createType(
      '(String, String, Result<(u64, H256), CheckpointError>)',
      reply.payload,
    );

    if (result.isErr) {
      const error = result.asErr.toString() as CheckpointError;

      if (error === 'NotPresent') {
        let unsub: () => void;
        logger.info(`Slot ${slot} hasn't been submitted yet. Subscribing for new slots`);

        const [_slot, _treeHashRoot] = await new Promise<[number, H256]>((resolve, reject) => {
          this._program.serviceSyncUpdate
            .subscribeToNewCheckpointEvent((event) => {
              logger.info(`Received new slot ${event.slot}`);
              if (event.slot >= slot) {
                resolve([event.slot, event.tree_hash_root]);
              }
            })
            .then((_unsub) => {
              unsub = _unsub;
            })
            .catch((e) => reject(e));
        }).finally(unsub!);

        return [_slot, _treeHashRoot];
      }
    }
    return [result.asOk[0].toNumber(), H256(result.asOk[1])];
  }
}

export class ServiceSyncUpdate {
  constructor(private _program: CheckpointClient) {}

  public subscribeToNewCheckpointEvent(
    callback: (data: { slot: number; tree_hash_root: H256 }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const _payload = message.payload.toHex();
      const service = getServiceNamePrefix(_payload, true);
      const method = getFnNamePrefix(_payload, true);

      if (service.service === 'ServiceSyncUpdate' && method.fn === 'NewCheckpoint') {
        const payload = this._program.registry.createType<NewCheckpointEventData>(
          '{"slot":"u64","tree_hash_root":"H256"}',
          message.payload.slice(service.bytesLength + method.bytesLength),
        );

        callback({ slot: payload['slot'].toNumber(), tree_hash_root: payload['tree_hash_root'].toHex() });
      }
    });
  }
}
