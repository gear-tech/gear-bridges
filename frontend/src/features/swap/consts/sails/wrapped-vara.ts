/* eslint-disable @typescript-eslint/no-floating-promises */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { GearApi, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import { TransactionBuilder, getServiceNamePrefix, getFnNamePrefix, ZERO_ADDRESS, ActorId } from 'sails-js';

export class Program {
  public readonly registry: TypeRegistry;
  public readonly tokenizer: Tokenizer;
  public readonly vft: Vft;

  constructor(public api: GearApi, public programId?: `0x${string}`) {
    const types: Record<string, any> = {};

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);

    this.tokenizer = new Tokenizer(this);
    this.vft = new Vft(this);
  }

  newCtorFromCode(code: Uint8Array | Buffer, name: string, symbol: string, decimals: number): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      ['New', name, symbol, decimals],
      '(String, String, String, u8)',
      'String',
      code,
    );

    this.programId = builder.programId;
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
    );

    this.programId = builder.programId;
    return builder;
  }
}

export class Tokenizer {
  constructor(private _program: Program) {}

  public burn(value: number | string | bigint): TransactionBuilder<bigint> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<bigint>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['Tokenizer', 'Burn', value],
      '(String, String, u128)',
      'u128',
      this._program.programId,
    );
  }

  public mint(): TransactionBuilder<bigint> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<bigint>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['Tokenizer', 'Mint'],
      '(String, String)',
      'u128',
      this._program.programId,
    );
  }

  public subscribeToMintedEvent(
    callback: (data: { to: ActorId; value: number | string | bigint }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'Tokenizer' && getFnNamePrefix(payload) === 'Minted') {
        callback(
          this._program.registry
            .createType('(String, String, {"to":"[u8;32]","value":"u128"})', message.payload)[2]
            .toJSON() as unknown as { to: ActorId; value: number | string | bigint },
        );
      }
    });
  }

  public subscribeToBurnedEvent(
    callback: (data: { from: ActorId; value: number | string | bigint }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'Tokenizer' && getFnNamePrefix(payload) === 'Burned') {
        callback(
          this._program.registry
            .createType('(String, String, {"from":"[u8;32]","value":"u128"})', message.payload)[2]
            .toJSON() as unknown as { from: ActorId; value: number | string | bigint },
        );
      }
    });
  }
}

export class Vft {
  constructor(private _program: Program) {}

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
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
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
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType('(String, String, U256)', reply.payload);
    return result[2].toBigInt() as unknown as bigint;
  }

  public async decimals(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<number> {
    const payload = this._program.registry.createType('(String, String)', ['Vft', 'Decimals']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType('(String, String, u8)', reply.payload);
    return result[2].toNumber() as unknown as number;
  }

  public async name(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<string> {
    const payload = this._program.registry.createType('(String, String)', ['Vft', 'Name']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType('(String, String, String)', reply.payload);
    return result[2].toString() as unknown as string;
  }

  public async symbol(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<string> {
    const payload = this._program.registry.createType('(String, String)', ['Vft', 'Symbol']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType('(String, String, String)', reply.payload);
    return result[2].toString() as unknown as string;
  }

  public async totalSupply(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<bigint> {
    const payload = this._program.registry.createType('(String, String)', ['Vft', 'TotalSupply']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
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
