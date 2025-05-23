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

export class SailsProgram {
  public readonly registry: TypeRegistry;
  public readonly vft: Vft;
  public readonly vftAdmin: VftAdmin;
  public readonly vftExtension: VftExtension;
  public readonly vftMetadata: VftMetadata;
  private _program!: Program;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {};

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new Program(programId, api);
    }

    this.vft = new Vft(this);
    this.vftAdmin = new VftAdmin(this);
    this.vftExtension = new VftExtension(this);
    this.vftMetadata = new VftMetadata(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }

  newCtorFromCode(
    code: Uint8Array | Buffer | HexString,
    name: string,
    symbol: string,
    decimals: number,
  ): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      ['New', name, symbol, decimals],
      '(String, String, String, u8)',
      'String',
      code,
      async (programId) => {
        this._program = await Program.new(programId, this.api);
      },
    );
    return builder;
  }

  newCtorFromCodeId(codeId: `0x${string}`, name: string, symbol: string, decimals: number) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      ['New', name, symbol, decimals],
      '(String, String, String, u8)',
      'String',
      codeId,
      async (programId) => {
        this._program = await Program.new(programId, this.api);
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
      ['Vft', 'Approve', spender, value],
      '(String, String, [u8;32], U256)',
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
      ['Vft', 'Transfer', to, value],
      '(String, String, [u8;32], U256)',
      'bool',
      this._program.programId,
    );
  }

  public transferFrom(from: ActorId, to: ActorId, value: number | string | bigint): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['Vft', 'TransferFrom', from, to, value],
      '(String, String, [u8;32], [u8;32], U256)',
      'bool',
      this._program.programId,
    );
  }

  public async allowance(
    owner: ActorId,
    spender: ActorId,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<bigint> {
    const payload = this._program.registry
      .createType('(String, String, [u8;32], [u8;32])', ['Vft', 'Allowance', owner, spender])
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
    const result = this._program.registry.createType('(String, String, U256)', reply.payload);
    return result[2].toBigInt() as unknown as bigint;
  }

  public async balanceOf(
    account: ActorId,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<bigint> {
    const payload = this._program.registry
      .createType('(String, String, [u8;32])', ['Vft', 'BalanceOf', account])
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
    const result = this._program.registry.createType('(String, String, U256)', reply.payload);
    return result[2].toBigInt() as unknown as bigint;
  }

  public async totalSupply(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<bigint> {
    const payload = this._program.registry.createType('(String, String)', ['Vft', 'TotalSupply']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, U256)', reply.payload);
    return result[2].toBigInt() as unknown as bigint;
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
      ['VftAdmin', 'AppendAllowancesShard', capacity],
      '(String, String, u32)',
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
      ['VftAdmin', 'AppendBalancesShard', capacity],
      '(String, String, u32)',
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
      ['VftAdmin', 'ApproveFrom', owner, spender, value],
      '(String, String, [u8;32], [u8;32], U256)',
      'bool',
      this._program.programId,
    );
  }

  public burn(from: ActorId, value: number | string | bigint): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftAdmin', 'Burn', from, value],
      '(String, String, [u8;32], U256)',
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
      ['VftAdmin', 'Exit', inheritor],
      '(String, String, [u8;32])',
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
      ['VftAdmin', 'Mint', to, value],
      '(String, String, [u8;32], U256)',
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
      ['VftAdmin', 'Pause'],
      '(String, String)',
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
      ['VftAdmin', 'Resume'],
      '(String, String)',
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
      ['VftAdmin', 'SetAdmin', admin],
      '(String, String, [u8;32])',
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
      ['VftAdmin', 'SetBurner', burner],
      '(String, String, [u8;32])',
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
      ['VftAdmin', 'SetExpiryPeriod', period],
      '(String, String, u32)',
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
      ['VftAdmin', 'SetMinimumBalance', value],
      '(String, String, U256)',
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
      ['VftAdmin', 'SetMinter', minter],
      '(String, String, [u8;32])',
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
      ['VftAdmin', 'SetPauser', pauser],
      '(String, String, [u8;32])',
      'Null',
      this._program.programId,
    );
  }

  public async admin(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftAdmin', 'Admin']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, [u8;32])', reply.payload);
    return result[2].toJSON() as unknown as ActorId;
  }

  public async burner(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftAdmin', 'Burner']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, [u8;32])', reply.payload);
    return result[2].toJSON() as unknown as ActorId;
  }

  public async isPaused(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<boolean> {
    const payload = this._program.registry.createType('(String, String)', ['VftAdmin', 'IsPaused']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, bool)', reply.payload);
    return result[2].toJSON() as unknown as boolean;
  }

  public async minter(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftAdmin', 'Minter']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, [u8;32])', reply.payload);
    return result[2].toJSON() as unknown as ActorId;
  }

  public async pauser(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftAdmin', 'Pauser']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, [u8;32])', reply.payload);
    return result[2].toJSON() as unknown as ActorId;
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
      ['VftExtension', 'AllocateNextAllowancesShard'],
      '(String, String)',
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
      ['VftExtension', 'AllocateNextBalancesShard'],
      '(String, String)',
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
      ['VftExtension', 'RemoveExpiredAllowance', owner, spender],
      '(String, String, [u8;32], [u8;32])',
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
      ['VftExtension', 'TransferAll', to],
      '(String, String, [u8;32])',
      'bool',
      this._program.programId,
    );
  }

  public transferAllFrom(from: ActorId, to: ActorId): TransactionBuilder<boolean> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<boolean>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftExtension', 'TransferAllFrom', from, to],
      '(String, String, [u8;32], [u8;32])',
      'bool',
      this._program.programId,
    );
  }

  public async allowanceOf(
    owner: ActorId,
    spender: ActorId,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<[number | string | bigint, number] | null> {
    const payload = this._program.registry
      .createType('(String, String, [u8;32], [u8;32])', ['VftExtension', 'AllowanceOf', owner, spender])
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
    const result = this._program.registry.createType('(String, String, Option<(U256, u32)>)', reply.payload);
    return result[2].toJSON() as unknown as [number | string | bigint, number] | null;
  }

  public async allowances(
    cursor: number,
    len: number,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[[ActorId, ActorId], [number | string | bigint, number]]>> {
    const payload = this._program.registry
      .createType('(String, String, u32, u32)', ['VftExtension', 'Allowances', cursor, len])
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
      '(String, String, Vec<(([u8;32], [u8;32]), (U256, u32))>)',
      reply.payload,
    );
    return result[2].toJSON() as unknown as Array<[[ActorId, ActorId], [number | string | bigint, number]]>;
  }

  public async balanceOf(
    account: ActorId,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<number | string | bigint | null> {
    const payload = this._program.registry
      .createType('(String, String, [u8;32])', ['VftExtension', 'BalanceOf', account])
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
    const result = this._program.registry.createType('(String, String, Option<U256>)', reply.payload);
    return result[2].toJSON() as unknown as number | string | bigint | null;
  }

  public async balances(
    cursor: number,
    len: number,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[ActorId, number | string | bigint]>> {
    const payload = this._program.registry
      .createType('(String, String, u32, u32)', ['VftExtension', 'Balances', cursor, len])
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
    const result = this._program.registry.createType('(String, String, Vec<([u8;32], U256)>)', reply.payload);
    return result[2].toJSON() as unknown as Array<[ActorId, number | string | bigint]>;
  }

  public async expiryPeriod(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<number> {
    const payload = this._program.registry.createType('(String, String)', ['VftExtension', 'ExpiryPeriod']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, u32)', reply.payload);
    return result[2].toNumber() as unknown as number;
  }

  public async minimumBalance(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<bigint> {
    const payload = this._program.registry.createType('(String, String)', ['VftExtension', 'MinimumBalance']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, U256)', reply.payload);
    return result[2].toBigInt() as unknown as bigint;
  }

  public async unusedValue(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<bigint> {
    const payload = this._program.registry.createType('(String, String)', ['VftExtension', 'UnusedValue']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, U256)', reply.payload);
    return result[2].toBigInt() as unknown as bigint;
  }
}

export class VftMetadata {
  constructor(private _program: SailsProgram) {}

  /**
   * Returns the number of decimals of the VFT.
   */
  public async decimals(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<number> {
    const payload = this._program.registry.createType('(String, String)', ['VftMetadata', 'Decimals']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, u8)', reply.payload);
    return result[2].toNumber() as unknown as number;
  }

  /**
   * Returns the name of the VFT.
   */
  public async name(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<string> {
    const payload = this._program.registry.createType('(String, String)', ['VftMetadata', 'Name']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, String)', reply.payload);
    return result[2].toString() as unknown as string;
  }

  /**
   * Returns the symbol of the VFT.
   */
  public async symbol(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<string> {
    const payload = this._program.registry.createType('(String, String)', ['VftMetadata', 'Symbol']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, String)', reply.payload);
    return result[2].toString() as unknown as string;
  }
}
