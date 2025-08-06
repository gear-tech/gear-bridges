/* eslint-disable @typescript-eslint/no-floating-promises */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { GearApi, Program, HexString, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import {
  TransactionBuilder,
  ActorId,
  throwOnErrorReply,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
} from 'sails-js';

interface State {
  /**
   * Admin of this service. Admin is in charge of:
   * - Changing fee
   * - Withdrawing collected fees from the program address
   * - Updating [State] of this service
   */
  admin_address: ActorId;
  /**
   * Fee amount that will be charged from users.
   */
  fee: number | string | bigint;
}

export class SailsProgram {
  public readonly registry: TypeRegistry;
  public readonly bridgingPayment: BridgingPayment;
  private _program!: Program;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      State: { admin_address: '[u8;32]', fee: 'u128' },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new Program(programId, api);
    }

    this.bridgingPayment = new BridgingPayment(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }

  /**
   * Create Bridging Payment program.
   */
  newCtorFromCode(code: Uint8Array | Buffer | HexString, initial_state: State): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      ['New', initial_state],
      '(String, State)',
      'String',
      code,
      async (programId) => {
        this._program = await Program.new(programId, this.api);
      },
    );
    return builder;
  }

  /**
   * Create Bridging Payment program.
   */
  newCtorFromCodeId(codeId: `0x${string}`, initial_state: State) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      ['New', initial_state],
      '(String, State)',
      'String',
      codeId,
      async (programId) => {
        this._program = await Program.new(programId, this.api);
      },
    );
    return builder;
  }
}

export class BridgingPayment {
  constructor(private _program: SailsProgram) {}

  /**
   * Pay fees for message processing to the admin.
   *
   * This method requires that **exactly** [Config::fee] must
   * be attached as a value when sending message to this method.
   *
   * Current fee amount can be retreived by calling `get_state`.
   */
  public payFees(nonce: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'PayFees', nonce],
      '(String, String, U256)',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Withdraw fees that were collected from user requests.
   *
   * This method can be called only by admin.
   */
  public reclaimFee(): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'ReclaimFee'],
      '(String, String)',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Set new admin.
   *
   * This method can be called only by admin.
   */
  public setAdmin(new_admin: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'SetAdmin', new_admin],
      '(String, String, [u8;32])',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Set fee that this program will take from incoming requests.
   *
   * This method can be called only by admin.
   */
  public setFee(fee: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'SetFee', fee],
      '(String, String, u128)',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Get current service [State].
   */
  public async getState(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<State> {
    const payload = this._program.registry.createType('(String, String)', ['BridgingPayment', 'GetState']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, State)', reply.payload);
    return result[2].toJSON() as unknown as State;
  }

  /**
   * Fee for the message processing by relayer was paid.
   */
  public subscribeToBridgingPaidEvent(
    callback: (data: { nonce: number | string | bigint }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'BridgingPayment' && getFnNamePrefix(payload) === 'BridgingPaid') {
        callback(
          this._program.registry
            .createType('(String, String, {"nonce":"U256"})', message.payload)[2]
            .toJSON() as unknown as { nonce: number | string | bigint },
        );
      }
    });
  }
}
