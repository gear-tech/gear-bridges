import { GearApi, BaseGearProgram, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import { H256, throwOnErrorReply, ZERO_ADDRESS } from 'sails-js';

export type CheckpointError = 'OutDated' | 'NotPresent';

export class CheckpointClient {
  public readonly registry: TypeRegistry;
  public readonly serviceCheckpointFor: ServiceCheckpointFor;
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
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }
}

export class ServiceCheckpointFor {
  constructor(private _program: CheckpointClient) {}

  public async get(
    slot: number | string | bigint,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<{ ok: [number | string | bigint, H256] } | { err: CheckpointError }> {
    const payload = this._program.registry
      .createType('(String, String, u64)', ['ServiceCheckpointFor', 'Get', slot])
      .toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType(
      '(String, String, Result<(u64, H256), CheckpointError>)',
      reply.payload,
    );
    return result[2].toJSON() as unknown as { ok: [number | string | bigint, H256] } | { err: CheckpointError };
  }
}
