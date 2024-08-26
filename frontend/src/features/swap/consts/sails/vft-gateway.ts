/* eslint-disable @typescript-eslint/no-floating-promises */
/* eslint-disable @typescript-eslint/no-explicit-any */

import { GearApi, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import { H160, ActorId, TransactionBuilder, MessageId, ZERO_ADDRESS } from 'sails-js';

export interface InitConfig {
  receiver_contract_address: H160;
  gear_bridge_builtin: ActorId;
  config: Config;
}

export interface Config {
  gas_to_burn_tokens: number | string | bigint;
  gas_for_reply_deposit: number | string | bigint;
  gas_to_mint_tokens: number | string | bigint;
  gas_to_send_request_to_builtin: number | string | bigint;
  reply_timeout: number;
  gas_for_transfer_to_eth_msg: number | string | bigint;
}

export type Error =
  | 'sendError'
  | 'replyError'
  | 'burnTokensDecodeError'
  | 'errorDuringTokensBurn'
  | 'requestToBuiltinSendError'
  | 'requestToBuiltinReplyError'
  | 'builtinDecodeError'
  | 'payloadSizeError'
  | 'mintTokensDecodeError'
  | 'replyTimeoutError'
  | 'errorDuringTokensMint'
  | 'noCorrespondingEthAddress'
  | 'replyHook'
  | 'messageNotFound'
  | 'invalidMessageStatus'
  | 'messageFailed'
  | 'burnTokensFailed'
  | 'bridgeBuiltinMessageFailed'
  | 'tokensRefundedError';

export interface MessageInfo {
  status: MessageStatus;
  details: TransactionDetails;
}

export type MessageStatus =
  | { sendingMessageToBurnTokens: null }
  | { tokenBurnCompleted: boolean }
  | { waitingReplyFromBurn: null }
  | { sendingMessageToBridgeBuiltin: null }
  | { bridgeResponseReceived: number | string | bigint | null }
  | { waitingReplyFromBuiltin: null }
  | { bridgeBuiltinStep: null }
  | { sendingMessageToMintTokens: null }
  | { tokenMintCompleted: null }
  | { waitingReplyFromMint: null }
  | { mintTokensStep: null }
  | { messageProcessedWithSuccess: number | string | bigint };

export interface TransactionDetails {
  vara_token_id: ActorId;
  sender: ActorId;
  amount: number | string | bigint;
  receiver: H160;
}

export class Program {
  public readonly registry: TypeRegistry;
  public readonly vftGateway: VftGateway;

  constructor(public api: GearApi, public programId?: `0x${string}`) {
    const types: Record<string, any> = {
      InitConfig: { receiver_contract_address: 'H160', gear_bridge_builtin: '[u8;32]', config: 'Config' },
      Config: {
        gas_to_burn_tokens: 'u64',
        gas_for_reply_deposit: 'u64',
        gas_to_mint_tokens: 'u64',
        gas_to_send_request_to_builtin: 'u64',
        reply_timeout: 'u32',
        gas_for_transfer_to_eth_msg: 'u64',
      },
      Error: {
        _enum: [
          'SendError',
          'ReplyError',
          'BurnTokensDecodeError',
          'ErrorDuringTokensBurn',
          'RequestToBuiltinSendError',
          'RequestToBuiltinReplyError',
          'BuiltinDecodeError',
          'PayloadSizeError',
          'MintTokensDecodeError',
          'ReplyTimeoutError',
          'ErrorDuringTokensMint',
          'NoCorrespondingEthAddress',
          'ReplyHook',
          'MessageNotFound',
          'InvalidMessageStatus',
          'MessageFailed',
          'BurnTokensFailed',
          'BridgeBuiltinMessageFailed',
          'TokensRefundedError',
        ],
      },
      MessageInfo: { status: 'MessageStatus', details: 'TransactionDetails' },
      MessageStatus: {
        _enum: {
          SendingMessageToBurnTokens: 'Null',
          TokenBurnCompleted: 'bool',
          WaitingReplyFromBurn: 'Null',
          SendingMessageToBridgeBuiltin: 'Null',
          BridgeResponseReceived: 'Option<U256>',
          WaitingReplyFromBuiltin: 'Null',
          BridgeBuiltinStep: 'Null',
          SendingMessageToMintTokens: 'Null',
          TokenMintCompleted: 'Null',
          WaitingReplyFromMint: 'Null',
          MintTokensStep: 'Null',
          MessageProcessedWithSuccess: 'U256',
        },
      },
      TransactionDetails: { vara_token_id: '[u8;32]', sender: '[u8;32]', amount: 'U256', receiver: 'H160' },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);

    this.vftGateway = new VftGateway(this);
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

export class VftGateway {
  constructor(private _program: Program) {}

  public handleInterruptedTransfer(
    msg_id: MessageId,
  ): TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftGateway', 'HandleInterruptedTransfer', msg_id],
      '(String, String, [u8;32])',
      'Result<(U256, H160), Error>',
      this._program.programId,
    );
  }

  public mapVaraToEthAddress(vara_token_id: ActorId, eth_token_id: H160): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftGateway', 'MapVaraToEthAddress', vara_token_id, eth_token_id],
      '(String, String, [u8;32], H160)',
      'Null',
      this._program.programId,
    );
  }

  public removeVaraToEthAddress(vara_token_id: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftGateway', 'RemoveVaraToEthAddress', vara_token_id],
      '(String, String, [u8;32])',
      'Null',
      this._program.programId,
    );
  }

  public transferVaraToEth(
    vara_token_id: ActorId,
    amount: number | string | bigint,
    receiver: H160,
  ): TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftGateway', 'TransferVaraToEth', vara_token_id, amount, receiver],
      '(String, String, [u8;32], U256, H160)',
      'Result<(U256, H160), Error>',
      this._program.programId,
    );
  }

  public updateConfig(
    gas_to_burn_tokens: number | string | bigint | null,
    gas_to_mint_tokens: number | string | bigint | null,
    gas_for_reply_deposit: number | string | bigint | null,
    gas_to_send_request_to_builtin: number | string | bigint | null,
    reply_timeout: number | null,
    gas_for_transfer_to_eth_msg: number | string | bigint | null,
  ): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      [
        'VftGateway',
        'UpdateConfig',
        gas_to_burn_tokens,
        gas_to_mint_tokens,
        gas_for_reply_deposit,
        gas_to_send_request_to_builtin,
        reply_timeout,
        gas_for_transfer_to_eth_msg,
      ],
      '(String, String, Option<u64>, Option<u64>, Option<u64>, Option<u64>, Option<u32>, Option<u64>)',
      'Null',
      this._program.programId,
    );
  }

  public updateReceiverContractAddress(new_receiver_contract_address: H160): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftGateway', 'UpdateReceiverContractAddress', new_receiver_contract_address],
      '(String, String, H160)',
      'Null',
      this._program.programId,
    );
  }

  public async admin(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftGateway', 'Admin']).toHex();
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

  public async gearBridgeBuiltin(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftGateway', 'GearBridgeBuiltin']).toHex();
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
    const payload = this._program.registry.createType('(String, String)', ['VftGateway', 'GetConfig']).toHex();
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
    const payload = this._program.registry.createType('(String, String)', ['VftGateway', 'MsgTrackerState']).toHex();
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

  public async receiverContractAddress(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<H160> {
    const payload = this._program.registry
      .createType('(String, String)', ['VftGateway', 'ReceiverContractAddress'])
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
    const result = this._program.registry.createType('(String, String, H160)', reply.payload);
    return result[2].toJSON() as unknown as H160;
  }

  public async varaToEthAddresses(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[ActorId, H160]>> {
    const payload = this._program.registry.createType('(String, String)', ['VftGateway', 'VaraToEthAddresses']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType('(String, String, Vec<([u8;32], H160)>)', reply.payload);
    return result[2].toJSON() as unknown as Array<[ActorId, H160]>;
  }
}
