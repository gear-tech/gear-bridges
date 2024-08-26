/* eslint-disable @typescript-eslint/no-floating-promises */
/* eslint-disable @typescript-eslint/no-explicit-any */

import { GearApi, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import {
  ActorId,
  H160,
  TransactionBuilder,
  MessageId,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
} from 'sails-js';

export interface InitConfig {
  admin_address: ActorId;
  vft_gateway_address: ActorId;
  config: Config;
}

export interface Config {
  fee: number | string | bigint;
  gas_for_reply_deposit: number | string | bigint;
  gas_to_send_request_to_gateway: number | string | bigint;
  gas_to_transfer_tokens: number | string | bigint;
  reply_timeout: number;
  gas_for_request_to_gateway_msg: number | string | bigint;
}

export interface MessageInfo {
  status: MessageStatus;
  details: TransactionDetails;
}

export type MessageStatus =
  | { sendingMessageToTransferTokens: null }
  | { tokenTransferCompleted: boolean }
  | { waitingReplyFromTokenTransfer: null }
  | { sendingMessageToGateway: null }
  | { gatewayMessageProcessingCompleted: [number | string | bigint, H160] }
  | { waitingReplyFromGateway: null }
  | { messageToGatewayStep: null }
  | { returnTokensBackStep: null }
  | { sendingMessageToTransferTokensBack: null }
  | { waitingReplyFromTokenTransferBack: null }
  | { tokenTransferBackCompleted: null }
  | { messageProcessedWithSuccess: [number | string | bigint, H160] };

export type TransactionDetails =
  | { transfer: { sender: ActorId; receiver: ActorId; amount: number | string | bigint; token_id: ActorId } }
  | {
      sendMessageToGateway: {
        sender: ActorId;
        vara_token_id: ActorId;
        amount: number | string | bigint;
        receiver: H160;
        attached_value: number | string | bigint;
      };
    };

export class Program {
  public readonly registry: TypeRegistry;
  public readonly bridgingPayment: BridgingPayment;

  constructor(public api: GearApi, public programId?: `0x${string}`) {
    const types: Record<string, any> = {
      InitConfig: { admin_address: '[u8;32]', vft_gateway_address: '[u8;32]', config: 'Config' },
      Config: {
        fee: 'u128',
        gas_for_reply_deposit: 'u64',
        gas_to_send_request_to_gateway: 'u64',
        gas_to_transfer_tokens: 'u64',
        reply_timeout: 'u32',
        gas_for_request_to_gateway_msg: 'u64',
      },
      MessageInfo: { status: 'MessageStatus', details: 'TransactionDetails' },
      MessageStatus: {
        _enum: {
          SendingMessageToTransferTokens: 'Null',
          TokenTransferCompleted: 'bool',
          WaitingReplyFromTokenTransfer: 'Null',
          SendingMessageToGateway: 'Null',
          GatewayMessageProcessingCompleted: '(U256, H160)',
          WaitingReplyFromGateway: 'Null',
          MessageToGatewayStep: 'Null',
          ReturnTokensBackStep: 'Null',
          SendingMessageToTransferTokensBack: 'Null',
          WaitingReplyFromTokenTransferBack: 'Null',
          TokenTransferBackCompleted: 'Null',
          MessageProcessedWithSuccess: '(U256, H160)',
        },
      },
      TransactionDetails: {
        _enum: {
          Transfer: { sender: '[u8;32]', receiver: '[u8;32]', amount: 'U256', token_id: '[u8;32]' },
          SendMessageToGateway: {
            sender: '[u8;32]',
            vara_token_id: '[u8;32]',
            amount: 'U256',
            receiver: 'H160',
            attached_value: 'u128',
          },
        },
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

  public continueTransaction(msg_id: MessageId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'ContinueTransaction', msg_id],
      '(String, String, [u8;32])',
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

  public requestToGateway(
    amount: number | string | bigint,
    receiver: H160,
    vara_token_id: ActorId,
  ): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'RequestToGateway', amount, receiver, vara_token_id],
      '(String, String, U256, H160, [u8;32])',
      'Null',
      this._program.programId,
    );
  }

  public returnTokens(msg_id: MessageId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'ReturnTokens', msg_id],
      '(String, String, [u8;32])',
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

  public updateConfig(
    fee: number | string | bigint | null,
    gas_for_reply_deposit: number | string | bigint | null,
    gas_to_send_request_to_gateway: number | string | bigint | null,
    gas_to_transfer_tokens: number | string | bigint | null,
    reply_timeout: number | null,
    gas_for_request_to_gateway_msg: number | string | bigint | null,
  ): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      [
        'BridgingPayment',
        'UpdateConfig',
        fee,
        gas_for_reply_deposit,
        gas_to_send_request_to_gateway,
        gas_to_transfer_tokens,
        reply_timeout,
        gas_for_request_to_gateway_msg,
      ],
      '(String, String, Option<u128>, Option<u64>, Option<u64>, Option<u64>, Option<u32>, Option<u64>)',
      'Null',
      this._program.programId,
    );
  }

  public updateVftGatewayAddress(new_vft_gateway_address: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['BridgingPayment', 'UpdateVftGatewayAddress', new_vft_gateway_address],
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

  public async msgTrackerState(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[MessageId, MessageInfo]>> {
    const payload = this._program.registry
      .createType('(String, String)', ['BridgingPayment', 'MsgTrackerState'])
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
    const result = this._program.registry.createType('(String, String, Vec<([u8;32], MessageInfo)>)', reply.payload);
    return result[2].toJSON() as unknown as Array<[MessageId, MessageInfo]>;
  }

  public async vftGatewayAddress(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry
      .createType('(String, String)', ['BridgingPayment', 'VftGatewayAddress'])
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
