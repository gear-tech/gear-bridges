/* eslint-disable */

import { GearApi, BaseGearProgram, HexString } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import {
  TransactionBuilder,
  ActorId,
  QueryBuilder,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
} from 'sails-js';

/**
 * Specifies the network for deployment of VFT-VARA
 */
export type Mainnet = 'Yes' | 'No';

export class SailsProgram {
  public readonly registry: TypeRegistry;
  public readonly vft: Vft;
  public readonly vftAdmin: VftAdmin;
  public readonly vftExtension: VftExtension;
  public readonly vftMetadata: VftMetadata;
  public readonly vftNativeExchange: VftNativeExchange;
  public readonly vftNativeExchangeAdmin: VftNativeExchangeAdmin;
  private _program?: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      Mainnet: { _enum: ['Yes', 'No'] },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
    }

    this.vft = new Vft(this);
    this.vftAdmin = new VftAdmin(this);
    this.vftExtension = new VftExtension(this);
    this.vftMetadata = new VftMetadata(this);
    this.vftNativeExchange = new VftNativeExchange(this);
    this.vftNativeExchangeAdmin = new VftNativeExchangeAdmin(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }

  newCtorFromCode(code: Uint8Array | Buffer | HexString, network: Mainnet): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      null,
      'New',
      network,
      'Mainnet',
      'String',
      code,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }

  newCtorFromCodeId(codeId: `0x${string}`, network: Mainnet) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      null,
      'New',
      network,
      'Mainnet',
      'String',
      codeId,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }
}

export class Vft {
  constructor(private _program: SailsProgram) {}

  public approve(spender: ActorId, value: number | string | bigint): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'Vft',
      'Approve',
      [spender, value],
      '([u8;32], U256)',
      'bool',
      this._program.programId,
    );
  }

  public transfer(to: ActorId, value: number | string | bigint): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'Vft',
      'Transfer',
      [to, value],
      '([u8;32], U256)',
      'bool',
      this._program.programId,
    );
  }

  public transferFrom($from: ActorId, to: ActorId, value: number | string | bigint): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'Vft',
      'TransferFrom',
      [$from, to, value],
      '([u8;32], [u8;32], U256)',
      'bool',
      this._program.programId,
    );
  }

  public allowance(owner: ActorId, spender: ActorId): QueryBuilder<bigint> {
    return new QueryBuilder<bigint>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'Vft',
      'Allowance',
      [owner, spender],
      '([u8;32], [u8;32])',
      'U256',
    );
  }

  public balanceOf(account: ActorId): QueryBuilder<bigint> {
    return new QueryBuilder<bigint>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'Vft',
      'BalanceOf',
      account,
      '[u8;32]',
      'U256',
    );
  }

  public totalSupply(): QueryBuilder<bigint> {
    return new QueryBuilder<bigint>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'Vft',
      'TotalSupply',
      null,
      null,
      'U256',
    );
  }

  public subscribeToApprovalEvent(
    callback: (data: { owner: ActorId; spender: ActorId; value: number | string | bigint }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'Vft' && getFnNamePrefix(payload) === 'Approval') {
        callback(
          this._program.registry
            .createType('(String, String, {"owner":"[u8;32]","spender":"[u8;32]","value":"U256"})', message.payload)[2]
            .toJSON() as unknown as { owner: ActorId; spender: ActorId; value: number | string | bigint },
        );
      }
    });
  }

  public subscribeToTransferEvent(
    callback: (data: { from: ActorId; to: ActorId; value: number | string | bigint }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'Vft' && getFnNamePrefix(payload) === 'Transfer') {
        callback(
          this._program.registry
            .createType('(String, String, {"from":"[u8;32]","to":"[u8;32]","value":"U256"})', message.payload)[2]
            .toJSON() as unknown as { from: ActorId; to: ActorId; value: number | string | bigint },
        );
      }
    });
  }
}

export class VftAdmin {
  constructor(private _program: SailsProgram) {}

  public appendAllowancesShard(capacity: number): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'AppendAllowancesShard',
      capacity,
      'u32',
      'Null',
      this._program.programId,
    );
  }

  public appendBalancesShard(capacity: number): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'AppendBalancesShard',
      capacity,
      'u32',
      'Null',
      this._program.programId,
    );
  }

  public approveFrom(owner: ActorId, spender: ActorId, value: number | string | bigint): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'ApproveFrom',
      [owner, spender, value],
      '([u8;32], [u8;32], U256)',
      'bool',
      this._program.programId,
    );
  }

  public burn($from: ActorId, value: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'Burn',
      [$from, value],
      '([u8;32], U256)',
      'Null',
      this._program.programId,
    );
  }

  public exit(inheritor: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'Exit',
      inheritor,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  public mint(to: ActorId, value: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'Mint',
      [to, value],
      '([u8;32], U256)',
      'Null',
      this._program.programId,
    );
  }

  public pause(): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'Pause',
      null,
      null,
      'Null',
      this._program.programId,
    );
  }

  public resume(): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'Resume',
      null,
      null,
      'Null',
      this._program.programId,
    );
  }

  public setAdmin(admin: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'SetAdmin',
      admin,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  public setBurner(burner: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'SetBurner',
      burner,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  public setExpiryPeriod(period: number): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'SetExpiryPeriod',
      period,
      'u32',
      'Null',
      this._program.programId,
    );
  }

  public setMinimumBalance(value: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'SetMinimumBalance',
      value,
      'U256',
      'Null',
      this._program.programId,
    );
  }

  public setMinter(minter: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'SetMinter',
      minter,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  public setPauser(pauser: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftAdmin',
      'SetPauser',
      pauser,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  public admin(): QueryBuilder<ActorId> {
    return new QueryBuilder<ActorId>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftAdmin',
      'Admin',
      null,
      null,
      '[u8;32]',
    );
  }

  public burner(): QueryBuilder<ActorId> {
    return new QueryBuilder<ActorId>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftAdmin',
      'Burner',
      null,
      null,
      '[u8;32]',
    );
  }

  public isPaused(): QueryBuilder<boolean> {
    return new QueryBuilder<boolean>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftAdmin',
      'IsPaused',
      null,
      null,
      'bool',
    );
  }

  public minter(): QueryBuilder<ActorId> {
    return new QueryBuilder<ActorId>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftAdmin',
      'Minter',
      null,
      null,
      '[u8;32]',
    );
  }

  public pauser(): QueryBuilder<ActorId> {
    return new QueryBuilder<ActorId>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftAdmin',
      'Pauser',
      null,
      null,
      '[u8;32]',
    );
  }

  public subscribeToAdminChangedEvent(callback: (data: ActorId) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'AdminChanged') {
        callback(
          this._program.registry
            .createType('(String, String, [u8;32])', message.payload)[2]
            .toJSON() as unknown as ActorId,
        );
      }
    });
  }

  public subscribeToBurnerChangedEvent(callback: (data: ActorId) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'BurnerChanged') {
        callback(
          this._program.registry
            .createType('(String, String, [u8;32])', message.payload)[2]
            .toJSON() as unknown as ActorId,
        );
      }
    });
  }

  public subscribeToMinterChangedEvent(callback: (data: ActorId) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'MinterChanged') {
        callback(
          this._program.registry
            .createType('(String, String, [u8;32])', message.payload)[2]
            .toJSON() as unknown as ActorId,
        );
      }
    });
  }

  public subscribeToPauserChangedEvent(callback: (data: ActorId) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'PauserChanged') {
        callback(
          this._program.registry
            .createType('(String, String, [u8;32])', message.payload)[2]
            .toJSON() as unknown as ActorId,
        );
      }
    });
  }

  public subscribeToBurnerTookPlaceEvent(callback: (data: null) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'BurnerTookPlace') {
        callback(null);
      }
    });
  }

  public subscribeToMinterTookPlaceEvent(callback: (data: null) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'MinterTookPlace') {
        callback(null);
      }
    });
  }

  public subscribeToExpiryPeriodChangedEvent(callback: (data: number) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'ExpiryPeriodChanged') {
        callback(
          this._program.registry
            .createType('(String, String, u32)', message.payload)[2]
            .toNumber() as unknown as number,
        );
      }
    });
  }

  public subscribeToMinimumBalanceChangedEvent(callback: (data: bigint) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'MinimumBalanceChanged') {
        callback(
          this._program.registry
            .createType('(String, String, U256)', message.payload)[2]
            .toBigInt() as unknown as bigint,
        );
      }
    });
  }

  public subscribeToExitedEvent(callback: (data: ActorId) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'Exited') {
        callback(
          this._program.registry
            .createType('(String, String, [u8;32])', message.payload)[2]
            .toJSON() as unknown as ActorId,
        );
      }
    });
  }

  public subscribeToPausedEvent(callback: (data: null) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'Paused') {
        callback(null);
      }
    });
  }

  public subscribeToResumedEvent(callback: (data: null) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftAdmin' && getFnNamePrefix(payload) === 'Resumed') {
        callback(null);
      }
    });
  }
}

export class VftExtension {
  constructor(private _program: SailsProgram) {}

  public allocateNextAllowancesShard(): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftExtension',
      'AllocateNextAllowancesShard',
      null,
      null,
      'bool',
      this._program.programId,
    );
  }

  public allocateNextBalancesShard(): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftExtension',
      'AllocateNextBalancesShard',
      null,
      null,
      'bool',
      this._program.programId,
    );
  }

  public removeExpiredAllowance(owner: ActorId, spender: ActorId): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftExtension',
      'RemoveExpiredAllowance',
      [owner, spender],
      '([u8;32], [u8;32])',
      'bool',
      this._program.programId,
    );
  }

  public transferAll(to: ActorId): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftExtension',
      'TransferAll',
      to,
      '[u8;32]',
      'bool',
      this._program.programId,
    );
  }

  public transferAllFrom($from: ActorId, to: ActorId): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftExtension',
      'TransferAllFrom',
      [$from, to],
      '([u8;32], [u8;32])',
      'bool',
      this._program.programId,
    );
  }

  public allowanceOf(owner: ActorId, spender: ActorId): QueryBuilder<[number | string | bigint, number] | null> {
    return new QueryBuilder<[number | string | bigint, number] | null>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftExtension',
      'AllowanceOf',
      [owner, spender],
      '([u8;32], [u8;32])',
      'Option<(U256, u32)>',
    );
  }

  public allowances(
    cursor: number,
    len: number,
  ): QueryBuilder<Array<[[ActorId, ActorId], [number | string | bigint, number]]>> {
    return new QueryBuilder<Array<[[ActorId, ActorId], [number | string | bigint, number]]>>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftExtension',
      'Allowances',
      [cursor, len],
      '(u32, u32)',
      'Vec<(([u8;32], [u8;32]), (U256, u32))>',
    );
  }

  public balanceOf(account: ActorId): QueryBuilder<number | string | bigint | null> {
    return new QueryBuilder<number | string | bigint | null>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftExtension',
      'BalanceOf',
      account,
      '[u8;32]',
      'Option<U256>',
    );
  }

  public balances(cursor: number, len: number): QueryBuilder<Array<[ActorId, number | string | bigint]>> {
    return new QueryBuilder<Array<[ActorId, number | string | bigint]>>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftExtension',
      'Balances',
      [cursor, len],
      '(u32, u32)',
      'Vec<([u8;32], U256)>',
    );
  }

  public expiryPeriod(): QueryBuilder<number> {
    return new QueryBuilder<number>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftExtension',
      'ExpiryPeriod',
      null,
      null,
      'u32',
    );
  }

  public minimumBalance(): QueryBuilder<bigint> {
    return new QueryBuilder<bigint>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftExtension',
      'MinimumBalance',
      null,
      null,
      'U256',
    );
  }

  public unusedValue(): QueryBuilder<bigint> {
    return new QueryBuilder<bigint>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftExtension',
      'UnusedValue',
      null,
      null,
      'U256',
    );
  }
}

export class VftMetadata {
  constructor(private _program: SailsProgram) {}

  /**
   * Returns the number of decimals of the VFT.
   */
  public decimals(): QueryBuilder<number> {
    return new QueryBuilder<number>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftMetadata',
      'Decimals',
      null,
      null,
      'u8',
    );
  }

  /**
   * Returns the name of the VFT.
   */
  public name(): QueryBuilder<string> {
    return new QueryBuilder<string>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftMetadata',
      'Name',
      null,
      null,
      'String',
    );
  }

  /**
   * Returns the symbol of the VFT.
   */
  public symbol(): QueryBuilder<string> {
    return new QueryBuilder<string>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'VftMetadata',
      'Symbol',
      null,
      null,
      'String',
    );
  }
}

export class VftNativeExchange {
  constructor(private _program: SailsProgram) {}

  public burn(value: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftNativeExchange',
      'Burn',
      value,
      'U256',
      'Null',
      this._program.programId,
    );
  }

  public burnAll(): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftNativeExchange',
      'BurnAll',
      null,
      null,
      'Null',
      this._program.programId,
    );
  }

  public mint(): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftNativeExchange',
      'Mint',
      null,
      null,
      'Null',
      this._program.programId,
    );
  }
}

export class VftNativeExchangeAdmin {
  constructor(private _program: SailsProgram) {}

  public burnFrom($from: ActorId, value: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftNativeExchangeAdmin',
      'BurnFrom',
      [$from, value],
      '([u8;32], U256)',
      'Null',
      this._program.programId,
    );
  }

  public subscribeToFailedMintEvent(
    callback: (data: { to: ActorId; value: number | string | bigint }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftNativeExchangeAdmin' && getFnNamePrefix(payload) === 'FailedMint') {
        callback(
          this._program.registry
            .createType('(String, String, {"to":"[u8;32]","value":"U256"})', message.payload)[2]
            .toJSON() as unknown as { to: ActorId; value: number | string | bigint },
        );
      }
    });
  }
}
