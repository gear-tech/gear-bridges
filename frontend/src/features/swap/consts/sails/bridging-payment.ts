/* eslint-disable @typescript-eslint/no-floating-promises */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { GearApi, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import { TransactionBuilder, H160, ActorId, getServiceNamePrefix, getFnNamePrefix, ZERO_ADDRESS } from 'sails-js';

export interface InitConfig {
  admin_address: ActorId;
  vft_manager_address: ActorId;
  config: Config;
}

export interface Config {
  fee: number | string | bigint;
  gas_for_reply_deposit: number | string | bigint;
  gas_to_send_request_to_vft_manager: number | string | bigint;
  reply_timeout: number;
  gas_for_request_to_vft_manager_msg: number | string | bigint;
}

export class Program {
  public readonly registry: TypeRegistry;
  public readonly bridgingPayment: BridgingPayment;

  constructor(
    public api: GearApi,
    public programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      InitConfig: { admin_address: '[u8;32]', vft_manager_address: '[u8;32]', config: 'Config' },
      Config: {
        fee: 'u128',
        gas_for_reply_deposit: 'u64',
        gas_to_send_request_to_vft_manager: 'u64',
        reply_timeout: 'u32',
        gas_for_request_to_vft_manager_msg: 'u64',
      },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);

    this.bridgingPayment = new BridgingPayment(this);
  }

  newCtorFromCode(code: Uint8Array | Buffer, init_config: InitConfig): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      ['New', init_config],
      '(String, InitConfig)',
      'String',
      code,
    );

    this.programId = builder.programId;
    return builder;
  }

  newCtorFromCodeId(codeId: `0x${string}`, init_config: InitConfig) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      ['New', init_config],
      '(String, InitConfig)',
      'String',
      codeId,
    );

    this.programId = builder.programId;
    return builder;
  }
}

export class BridgingPayment {
  constructor(private _program: Program) {}

  public makeRequest(
    amount: number | string | bigint,
    receiver: H160,
    vara_token_id: ActorId,
  ): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'MakeRequest', amount, receiver, vara_token_id],
      '(String, String, U256, H160, [u8;32])',
      'Null',
      this._program.programId,
    );
  }

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

  public setConfig(config: Config): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'SetConfig', config],
      '(String, String, Config)',
      'Null',
      this._program.programId,
    );
  }

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

  public updateVftManagerAddress(new_vft_manager_address: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'UpdateVftManagerAddress', new_vft_manager_address],
      '(String, String, [u8;32])',
      'Null',
      this._program.programId,
    );
  }

  public async adminAddress(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['BridgingPayment', 'AdminAddress']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType('(String, String, [u8;32])', reply.payload);
    return result[2].toJSON() as unknown as ActorId;
  }

  public async getConfig(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Config> {
    const payload = this._program.registry.createType('(String, String)', ['BridgingPayment', 'GetConfig']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType('(String, String, Config)', reply.payload);
    return result[2].toJSON() as unknown as Config;
  }

  public async vftManagerAddress(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry
      .createType('(String, String)', ['BridgingPayment', 'VftManagerAddress'])
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
    const result = this._program.registry.createType('(String, String, [u8;32])', reply.payload);
    return result[2].toJSON() as unknown as ActorId;
  }

  public subscribeToTeleportVaraToEthEvent(
    callback: (data: {
      nonce: number | string | bigint;
      sender: ActorId;
      amount: number | string | bigint;
      receiver: H160;
      eth_token_id: H160;
    }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'BridgingPayment' && getFnNamePrefix(payload) === 'TeleportVaraToEth') {
        callback(
          this._program.registry
            .createType(
              '(String, String, {"nonce":"U256","sender":"[u8;32]","amount":"U256","receiver":"H160","eth_token_id":"H160"})',
              message.payload,
            )[2]
            .toJSON() as unknown as {
            nonce: number | string | bigint;
            sender: ActorId;
            amount: number | string | bigint;
            receiver: H160;
            eth_token_id: H160;
          },
        );
      }
    });
  }
}
