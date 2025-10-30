import { GearApi, BaseGearProgram } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import { ActorId, QueryBuilder } from 'sails-js';

export class EthEventsClient {
  public readonly registry: TypeRegistry;
  public readonly ethereumEventClient: EthereumEventClient;
  private _program?: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
    }
    this.registry = new TypeRegistry();
    this.ethereumEventClient = new EthereumEventClient(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }
}

export class EthereumEventClient {
  constructor(private _program: EthEventsClient) {}

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
