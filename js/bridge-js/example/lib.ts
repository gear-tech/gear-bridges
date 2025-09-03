import { GearApi, BaseGearProgram, HexString } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import { TransactionBuilder, getServiceNamePrefix, getFnNamePrefix, ZERO_ADDRESS } from 'sails-js';

export class PingClient {
  public readonly registry: TypeRegistry;
  public readonly ping: Ping;
  private _program: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {};

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
    }

    this.ping = new Ping(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }

  newCtorFromCode(code: Uint8Array | Buffer | HexString): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      undefined,
      'New',
      undefined,
      '()',
      'String',
      code,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }
}

export class Ping {
  constructor(private _program: PingClient) {}

  public submitReceipt(
    slot: number | string | bigint,
    transaction_index: number,
    _receipt_rlp: `0x${string}`,
  ): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'Ping',
      'SubmitReceipt',
      [slot, transaction_index, _receipt_rlp],
      '(u64, u32, Vec<u8>)',
      'Null',
      this._program.programId,
    );
  }

  public subscribeToReceiptSubmittedEvent(
    callback: (data: [number | string | bigint, number]) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'Ping' && getFnNamePrefix(payload) === 'ReceiptSubmitted') {
        callback(
          this._program.registry.createType('(String, String, (u64, u32))', message.payload)[2].toJSON() as unknown as [
            number | string | bigint,
            number,
          ],
        );
      }
    });
  }
}
