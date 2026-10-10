import { GearApi } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import { ActorId, QueryBuilder } from 'sails-js';

export class EthEventsClient {
  public readonly registry: TypeRegistry;
  public readonly ethereumEventClient: EthereumEventClient;

  constructor(
    public api: GearApi,
    private readonly _programId: `0x${string}`,
  ) {
    this.registry = new TypeRegistry();
    this.ethereumEventClient = new EthereumEventClient(this);
  }

  public get programId(): `0x${string}` {
    return this._programId;
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
