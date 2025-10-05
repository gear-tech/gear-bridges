/* eslint-disable */

import {
  ActorId,
  TransactionBuilder,
  H256,
  QueryBuilder,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
} from 'sails-js';
import { GearApi, BaseGearProgram, HexString } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';

/**
 * Global state of the Bridging Payment service.
 */
export interface State {
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
  /**
   * Priority fee amount that will be charged from users.
   */
  priority_fee: number | string | bigint;
}

export class SailsProgram {
  public readonly registry: TypeRegistry;
  public readonly bridgingPayment: BridgingPayment;
  private _program?: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      State: { admin_address: '[u8;32]', fee: 'u128', priority_fee: 'u128' },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
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
      null,
      'New',
      initial_state,
      'State',
      'String',
      code,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
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
      null,
      'New',
      initial_state,
      'State',
      'String',
      codeId,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
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
   * This method requires that **exactly** [State::fee] must
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
      'BridgingPayment',
      'PayFees',
      nonce,
      'U256',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Pay fees for priority message processing to the admin.
   *
   * This method requires that **exactly** [State::priority_fee] must be
   * attached as a value when sending message to this method.
   *
   * Current fee amount can be retrieved by calling `get_state`.
   */
  public payPriorityFees(block: H256, nonce: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'BridgingPayment',
      'PayPriorityFees',
      [block, nonce],
      '(H256, U256)',
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
      'BridgingPayment',
      'ReclaimFee',
      null,
      null,
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
      'BridgingPayment',
      'SetAdmin',
      new_admin,
      '[u8;32]',
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
      'BridgingPayment',
      'SetFee',
      fee,
      'u128',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Set fee that this program will take for processing priority
   * requests.
   *
   * This method can be called only by admin.
   */
  public setPriorityFee(priority_fee: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'BridgingPayment',
      'SetPriorityFee',
      priority_fee,
      'u128',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Upgrades the program to the provided new address.
   */
  public upgrade($new: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'BridgingPayment',
      'Upgrade',
      $new,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Get current service [State].
   */
  public getState(): QueryBuilder<State> {
    return new QueryBuilder<State>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'BridgingPayment',
      'GetState',
      null,
      null,
      'State',
    );
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

  /**
   * Fee for the message processing by relayer was paid
   * and priority bridging was requested.
   */
  public subscribeToPriorityBridgingPaidEvent(
    callback: (data: { block: H256; nonce: number | string | bigint }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'BridgingPayment' && getFnNamePrefix(payload) === 'PriorityBridgingPaid') {
        callback(
          this._program.registry
            .createType('(String, String, {"block":"H256","nonce":"U256"})', message.payload)[2]
            .toJSON() as unknown as { block: H256; nonce: number | string | bigint },
        );
      }
    });
  }
}
