/* eslint-disable @typescript-eslint/no-floating-promises */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { GearApi, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import {
  TransactionBuilder,
  H160,
  MessageId,
  ActorId,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
} from 'sails-js';

export interface InitConfig {
  erc20_manager_address: H160;
  gear_bridge_builtin: ActorId;
  historical_proxy_address: ActorId;
  config: Config;
}

export interface Config {
  gas_for_token_ops: number | string | bigint;
  gas_for_reply_deposit: number | string | bigint;
  gas_for_submit_receipt: number | string | bigint;
  gas_to_send_request_to_builtin: number | string | bigint;
  reply_timeout: number;
  gas_for_request_bridging: number | string | bigint;
}

export type Error =
  | 'sendFailure'
  | 'replyFailure'
  | 'burnTokensDecode'
  | 'transferFromDecode'
  | 'builtinDecode'
  | 'mintTokensDecode'
  | 'replyTimeout'
  | 'noCorrespondingEthAddress'
  | 'replyHook'
  | 'messageNotFound'
  | 'invalidMessageStatus'
  | 'messageFailed'
  | 'burnTokensFailed'
  | 'lockTokensFailed'
  | 'bridgeBuiltinMessageFailed'
  | 'tokensRefunded'
  | 'notEthClient'
  | 'notEnoughGas'
  | 'noCorrespondingVaraAddress'
  | 'notSupportedEvent';

export type TokenSupply = 'ethereum' | 'gear';

export interface MessageInfo {
  status: MessageStatus;
  details: TxDetails;
}

export type MessageStatus =
  | { sendingMessageToBridgeBuiltin: null }
  | { bridgeResponseReceived: number | string | bigint | null }
  | { waitingReplyFromBuiltin: null }
  | { bridgeBuiltinStep: null }
  | { sendingMessageToBurnTokens: null }
  | { tokenBurnCompleted: boolean }
  | { waitingReplyFromBurn: null }
  | { sendingMessageToMintTokens: null }
  | { tokenMintCompleted: null }
  | { waitingReplyFromMint: null }
  | { mintTokensStep: null }
  | { sendingMessageToLockTokens: null }
  | { tokenLockCompleted: boolean }
  | { waitingReplyFromLock: null }
  | { sendingMessageToUnlockTokens: null }
  | { tokenUnlockCompleted: null }
  | { waitingReplyFromUnlock: null }
  | { unlockTokensStep: null }
  | { messageProcessedWithSuccess: number | string | bigint };

export type TxDetails =
  | { requestBridging: { vara_token_id: ActorId; sender: ActorId; amount: number | string | bigint; receiver: H160 } }
  | { submitReceipt: { vara_token_id: ActorId; receiver: ActorId; amount: number | string | bigint } };

export class Program {
  public readonly registry: TypeRegistry;
  public readonly vftManager: VftManager;

  constructor(public api: GearApi, public programId?: `0x${string}`) {
    const types: Record<string, any> = {
      InitConfig: {
        erc20_manager_address: 'H160',
        gear_bridge_builtin: '[u8;32]',
        historical_proxy_address: '[u8;32]',
        config: 'Config',
      },
      Config: {
        gas_for_token_ops: 'u64',
        gas_for_reply_deposit: 'u64',
        gas_for_submit_receipt: 'u64',
        gas_to_send_request_to_builtin: 'u64',
        reply_timeout: 'u32',
        gas_for_request_bridging: 'u64',
      },
      Error: {
        _enum: [
          'SendFailure',
          'ReplyFailure',
          'BurnTokensDecode',
          'TransferFromDecode',
          'BuiltinDecode',
          'MintTokensDecode',
          'ReplyTimeout',
          'NoCorrespondingEthAddress',
          'ReplyHook',
          'MessageNotFound',
          'InvalidMessageStatus',
          'MessageFailed',
          'BurnTokensFailed',
          'LockTokensFailed',
          'BridgeBuiltinMessageFailed',
          'TokensRefunded',
          'NotEthClient',
          'NotEnoughGas',
          'NoCorrespondingVaraAddress',
          'NotSupportedEvent',
        ],
      },
      TokenSupply: { _enum: ['Ethereum', 'Gear'] },
      MessageInfo: { status: 'MessageStatus', details: 'TxDetails' },
      MessageStatus: {
        _enum: {
          SendingMessageToBridgeBuiltin: 'Null',
          BridgeResponseReceived: 'Option<U256>',
          WaitingReplyFromBuiltin: 'Null',
          BridgeBuiltinStep: 'Null',
          SendingMessageToBurnTokens: 'Null',
          TokenBurnCompleted: 'bool',
          WaitingReplyFromBurn: 'Null',
          SendingMessageToMintTokens: 'Null',
          TokenMintCompleted: 'Null',
          WaitingReplyFromMint: 'Null',
          MintTokensStep: 'Null',
          SendingMessageToLockTokens: 'Null',
          TokenLockCompleted: 'bool',
          WaitingReplyFromLock: 'Null',
          SendingMessageToUnlockTokens: 'Null',
          TokenUnlockCompleted: 'Null',
          WaitingReplyFromUnlock: 'Null',
          UnlockTokensStep: 'Null',
          MessageProcessedWithSuccess: 'U256',
        },
      },
      TxDetails: {
        _enum: {
          RequestBridging: { vara_token_id: '[u8;32]', sender: '[u8;32]', amount: 'U256', receiver: 'H160' },
          SubmitReceipt: { vara_token_id: '[u8;32]', receiver: '[u8;32]', amount: 'U256' },
        },
      },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);

    this.vftManager = new VftManager(this);
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

export class VftManager {
  constructor(private _program: Program) {}

  public handleInterruptedTransfer(
    msg_id: MessageId,
  ): TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftManager', 'HandleInterruptedTransfer', msg_id],
      '(String, String, [u8;32])',
      'Result<(U256, H160), Error>',
      this._program.programId,
    );
  }

  public mapVaraToEthAddress(
    vara_token_id: ActorId,
    eth_token_id: H160,
    supply_type: TokenSupply,
  ): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftManager', 'MapVaraToEthAddress', vara_token_id, eth_token_id, supply_type],
      '(String, String, [u8;32], H160, TokenSupply)',
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
      ['VftManager', 'RemoveVaraToEthAddress', vara_token_id],
      '(String, String, [u8;32])',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Request bridging of tokens from gear to ethereum. It involves locking/burning
   * `vft` tokens (specific operation depends on the token supply type) and sending
   * request to the bridge built-in actor.
   */
  public requestBridging(
    sender: ActorId,
    vara_token_id: ActorId,
    amount: number | string | bigint,
    receiver: H160,
  ): TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftManager', 'RequestBridging', sender, vara_token_id, amount, receiver],
      '(String, String, [u8;32], [u8;32], U256, H160)',
      'Result<(U256, H160), Error>',
      this._program.programId,
    );
  }

  /**
   * Submit rlp-encoded transaction receipt. This receipt is decoded under the hood
   * and checked that it's a valid receipt from tx send to `ERC20Manager` contract.
   * This entrypoint can be called only by `ethereum-event-client`.
   */
  public submitReceipt(receipt_rlp: `0x${string}`): TransactionBuilder<{ ok: null } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: null } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftManager', 'SubmitReceipt', receipt_rlp],
      '(String, String, Vec<u8>)',
      'Result<Null, Error>',
      this._program.programId,
    );
  }

  public updateConfig(config: Config): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftManager', 'UpdateConfig', config],
      '(String, String, Config)',
      'Null',
      this._program.programId,
    );
  }

  public updateErc20ManagerAddress(new_erc20_manager_address: H160): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftManager', 'UpdateErc20ManagerAddress', new_erc20_manager_address],
      '(String, String, H160)',
      'Null',
      this._program.programId,
    );
  }

  public updateEthClient(eth_client_new: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      ['VftManager', 'UpdateEthClient', eth_client_new],
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
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'Admin']).toHex();
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

  public async erc20ManagerAddress(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<H160> {
    const payload = this._program.registry
      .createType('(String, String)', ['VftManager', 'Erc20ManagerAddress'])
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

  public async ethClient(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'EthClient']).toHex();
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
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'GearBridgeBuiltin']).toHex();
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
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'GetConfig']).toHex();
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
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'MsgTrackerState']).toHex();
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

  public async varaToEthAddresses(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[ActorId, H160, TokenSupply]>> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'VaraToEthAddresses']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId!,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    if (!reply.code.isSuccess) throw new Error(this._program.registry.createType('String', reply.payload).toString());
    const result = this._program.registry.createType(
      '(String, String, Vec<([u8;32], H160, TokenSupply)>)',
      reply.payload,
    );
    return result[2].toJSON() as unknown as Array<[ActorId, H160, TokenSupply]>;
  }

  public subscribeToTokenMappingAddedEvent(
    callback: (data: { vara_token_id: ActorId; eth_token_id: H160 }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'TokenMappingAdded') {
        callback(
          this._program.registry
            .createType('(String, String, {"vara_token_id":"[u8;32]","eth_token_id":"H160"})', message.payload)[2]
            .toJSON() as unknown as { vara_token_id: ActorId; eth_token_id: H160 },
        );
      }
    });
  }

  public subscribeToTokenMappingRemovedEvent(
    callback: (data: { vara_token_id: ActorId; eth_token_id: H160 }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'TokenMappingRemoved') {
        callback(
          this._program.registry
            .createType('(String, String, {"vara_token_id":"[u8;32]","eth_token_id":"H160"})', message.payload)[2]
            .toJSON() as unknown as { vara_token_id: ActorId; eth_token_id: H160 },
        );
      }
    });
  }

  public subscribeToBridgingRequestedEvent(
    callback: (data: {
      nonce: number | string | bigint;
      vara_token_id: ActorId;
      amount: number | string | bigint;
      sender: ActorId;
      receiver: H160;
    }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'BridgingRequested') {
        callback(
          this._program.registry
            .createType(
              '(String, String, {"nonce":"U256","vara_token_id":"[u8;32]","amount":"U256","sender":"[u8;32]","receiver":"H160"})',
              message.payload,
            )[2]
            .toJSON() as unknown as {
            nonce: number | string | bigint;
            vara_token_id: ActorId;
            amount: number | string | bigint;
            sender: ActorId;
            receiver: H160;
          },
        );
      }
    });
  }
}
