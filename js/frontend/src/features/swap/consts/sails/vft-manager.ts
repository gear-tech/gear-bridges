/* eslint-disable @typescript-eslint/no-floating-promises */
/* eslint-disable @typescript-eslint/no-explicit-any */
import { GearApi, BaseGearProgram, HexString, decodeAddress } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import {
  TransactionBuilder,
  MessageId,
  ActorId,
  H160,
  throwOnErrorReply,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
} from 'sails-js';

export interface InitConfig {
  /**
   * Address of the gear-eth-bridge built-in actor.
   */
  gear_bridge_builtin: ActorId;
  /**
   * Address of the `historical-proxy` program.
   *
   * For more info see [State::historical_proxy_address].
   */
  historical_proxy_address: ActorId;
  /**
   * Config that will be used to send messages to the other programs.
   *
   * For more info see [Config].
   */
  config: Config;
}

/**
 * Config that will be used to send messages to the other programs.
 */
export interface Config {
  /**
   * Gas limit for token operations. Token operations include:
   * - Mint
   * - Burn
   * - TransferFrom
   */
  gas_for_token_ops: number | string | bigint;
  /**
   * Gas to reserve for reply processing.
   */
  gas_for_reply_deposit: number | string | bigint;
  /**
   * Gas limit for gear-eth-bridge built-in actor request.
   */
  gas_to_send_request_to_builtin: number | string | bigint;
  /**
   * Required gas to commit changes in [VftManager::update_vfts].
   */
  gas_for_swap_token_maps: number | string | bigint;
  /**
   * Timeout in blocks that current program will wait for reply from
   * the other programs such as VFT and `gear-eth-bridge` built-in actor.
   */
  reply_timeout: number;
  /**
   * Fee to pay `gear-eth-bridge` built-in actor.
   */
  fee_bridge: number | string | bigint;
  /**
   * Incoming fee.
   */
  fee_incoming: number | string | bigint;
}

/**
 * Type of the token supply.
 */
export type TokenSupply = 'Ethereum' | 'Gear';

/**
 * Error types for VFT Manageer service.
 */
export type Error =
  /**
   * Error sending message to the program.
   */
  | { SendFailure: string }
  /**
   * Error while waiting for reply from the program.
   */
  | { ReplyFailure: string }
  /**
   * Failed to set reply timeout.
   */
  | { ReplyTimeout: string }
  /**
   * Failed to set reply hook.
   */
  | { ReplyHook: string }
  /**
   * A message does not have a reply code.
   */
  | { NoReplyCode: string }
  /**
   * Original `MessageId` wasn't found in message tracker when processing reply.
   */
  | { MessageNotFound: null }
  /**
   * Invalid message status was found in the message tracker when processing reply.
   */
  | { InvalidMessageStatus: null }
  /**
   * Message sent to the program failed.
   */
  | { MessageFailed: null }
  /**
   * Failed to decode Burn reply.
   */
  | { BurnTokensDecode: string }
  /**
   * Failed to decode TransferFrom reply.
   */
  | { TransferFromDecode: string }
  /**
   * Failed to decode Mint reply.
   */
  | { MintTokensDecode: string }
  /**
   * Failed to decode payload from gear-eth-bridge built-in actor.
   */
  | { BuiltinDecode: string }
  /**
   * Gas reservation for reply is too low.
   */
  | { GasForReplyTooLow: string }
  /**
   * `ERC20` address wasn't found in the token mapping.
   */
  | { NoCorrespondingEthAddress: null }
  /**
   * `VFT` address wasn't found in the token mapping.
   */
  | { NoCorrespondingVaraAddress: null }
  /**
   * `submit_receipt` can only be called by `historical-proxy` program.
   */
  | { NotHistoricalProxy: null }
  /**
   * Ethereum transaction receipt is not supported.
   */
  | { UnsupportedEthEvent: null }
  /**
   * Ethereum transaction is too old and already have been removed from storage.
   */
  | { TransactionTooOld: null }
  /**
   * Ethereum transaction was already processed by VFT Manager service.
   */
  | { AlreadyProcessed: null }
  /**
   * Vft-manager is paused and cannot process the request.
   */
  | { Paused: null }
  /**
   * Failed to burn tokens from the receiver in VftVara.
   */
  | { BurnFromFailed: string }
  /**
   * Internal unspecified VFT error
   */
  | { Internal: string }
  /**
   * Invalid or unexpected reply received from a VFT program.
   */
  | { InvalidReply: null };

/**
 * State in which message processing can be.
 */
export type MessageStatus =
  /**
   * Message to deposit tokens is sent.
   */
  | { SendingMessageToDepositTokens: null }
  /**
   * Reply is received for a token deposit message.
   */
  | { TokenDepositCompleted: boolean }
  /**
   * Message to the `pallet-gear-eth-bridge` is sent.
   */
  | { SendingMessageToBridgeBuiltin: null }
  /**
   * Reply is received for a message to the `pallet-gear-eth-bridge`.
   */
  | { BridgeResponseReceived: number | string | bigint | null }
  /**
   * Message to refund tokens is sent.
   */
  | { SendingMessageToReturnTokens: null }
  /**
   * Reply is received for a token refund message.
   */
  | { TokensReturnComplete: boolean };

/**
 * Details about a request associated with a message stored in [MessageTracker].
 */
export interface TxDetails {
  /**
   * Address of the `VFT` token which is being bridged.
   */
  vara_token_id: ActorId;
  /**
   * Original `VFT` token owner.
   */
  sender: ActorId;
  /**
   * Bridged tokens amount.
   */
  amount: number | string | bigint;
  /**
   * `ERC20` token receiver on Ethereum.
   */
  receiver: H160;
  /**
   * [TokenSupply] type of the token being bridged.
   */
  token_supply: TokenSupply;
}

/**
 * Entry for a single message in [MessageTracker].
 */
export interface MessageInfo {
  /**
   * State of the message.
   */
  status: MessageStatus;
  /**
   * Request details.
   */
  details: TxDetails;
}

export type Order = 'Direct' | 'Reverse';

export class SailsProgram {
  public readonly registry: TypeRegistry;
  public readonly vftManager: VftManager;
  private _program!: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId?: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      InitConfig: { gear_bridge_builtin: '[u8;32]', historical_proxy_address: '[u8;32]', config: 'Config' },
      Config: {
        gas_for_token_ops: 'u64',
        gas_for_reply_deposit: 'u64',
        gas_to_send_request_to_builtin: 'u64',
        gas_for_swap_token_maps: 'u64',
        reply_timeout: 'u32',
        fee_bridge: 'u128',
        fee_incoming: 'u128',
      },
      TokenSupply: { _enum: ['Ethereum', 'Gear'] },
      Error: {
        _enum: {
          SendFailure: 'String',
          ReplyFailure: 'String',
          ReplyTimeout: 'String',
          ReplyHook: 'String',
          NoReplyCode: 'String',
          MessageNotFound: 'Null',
          InvalidMessageStatus: 'Null',
          MessageFailed: 'Null',
          BurnTokensDecode: 'String',
          TransferFromDecode: 'String',
          MintTokensDecode: 'String',
          BuiltinDecode: 'String',
          GasForReplyTooLow: 'String',
          NoCorrespondingEthAddress: 'Null',
          NoCorrespondingVaraAddress: 'Null',
          NotHistoricalProxy: 'Null',
          UnsupportedEthEvent: 'Null',
          TransactionTooOld: 'Null',
          AlreadyProcessed: 'Null',
          Paused: 'Null',
          BurnFromFailed: 'String',
          Internal: 'String',
          InvalidReply: 'Null',
        },
      },
      MessageStatus: {
        _enum: {
          SendingMessageToDepositTokens: 'Null',
          TokenDepositCompleted: 'bool',
          SendingMessageToBridgeBuiltin: 'Null',
          BridgeResponseReceived: 'Option<U256>',
          SendingMessageToReturnTokens: 'Null',
          TokensReturnComplete: 'bool',
        },
      },
      TxDetails: {
        vara_token_id: '[u8;32]',
        sender: '[u8;32]',
        amount: 'U256',
        receiver: 'H160',
        token_supply: 'TokenSupply',
      },
      MessageInfo: { status: 'MessageStatus', details: 'TxDetails' },
      Order: { _enum: ['Direct', 'Reverse'] },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    if (programId) {
      this._program = new BaseGearProgram(programId, api);
    }

    this.vftManager = new VftManager(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }

  /**
   * The constructor is intended for test purposes and is available only when the feature
   * `mocks` is enabled.
   */
  gasCalculationCtorFromCode(
    code: Uint8Array | Buffer | HexString,
    _init_config: InitConfig,
    _slot_first: number | string | bigint,
    _count: number | null,
  ): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      undefined,
      'GasCalculation',
      [_init_config, _slot_first, _count],
      '(InitConfig, u64, Option<u32>)',
      'String',
      code,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }

  /**
   * The constructor is intended for test purposes and is available only when the feature
   * `mocks` is enabled.
   */
  gasCalculationCtorFromCodeId(
    codeId: `0x${string}`,
    _init_config: InitConfig,
    _slot_first: number | string | bigint,
    _count: number | null,
  ) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      undefined,
      'GasCalculation',
      [_init_config, _slot_first, _count],
      '(InitConfig, u64, Option<u32>)',
      'String',
      codeId,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }
  newCtorFromCode(code: Uint8Array | Buffer | HexString, init_config: InitConfig): TransactionBuilder<null> {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'upload_program',
      undefined,
      'New',
      init_config,
      'InitConfig',
      'String',
      code,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }

  newCtorFromCodeId(codeId: `0x${string}`, init_config: InitConfig) {
    const builder = new TransactionBuilder<null>(
      this.api,
      this.registry,
      'create_program',
      undefined,
      'New',
      init_config,
      'InitConfig',
      'String',
      codeId,
      async (programId) => {
        this._program = await BaseGearProgram.new(programId, this.api);
      },
    );
    return builder;
  }
}

export class VftManager {
  constructor(private _program: SailsProgram) {}

  /**
   * The method is intended for tests and is available only when the feature `mocks`
   * is enabled. Sends a VFT-message to the sender to mint/unlock tokens depending
   * on the `_supply_type`.
   *
   * Designed for benchmarking gas consumption by the VFT-response processing function.
   */
  public calculateGasForReply(
    _slot: number | string | bigint,
    _transaction_index: number | string | bigint,
    _supply_type: TokenSupply,
  ): TransactionBuilder<{ ok: null } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: null } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'CalculateGasForReply',
      [_slot, _transaction_index, _supply_type],
      '(u64, u64, TokenSupply)',
      'Result<Null, Error>',
      this._program.programId,
    );
  }

  /**
   * Process message further if some error was encountered during the `request_bridging`.
   *
   * This method should be called only to recover funds that were stuck in the middle of the bridging
   * and is not a part of a normal workflow.
   *
   * There can be several reasons for `request_bridging` to fail:
   * - Gas attached to a message wasn't enough to execute entire logic in `request_bridging`.
   * - Network was heavily loaded and some message was stuck so `request_bridging` failed.
   */
  public handleRequestBridgingInterruptedTransfer(
    msg_id: MessageId,
  ): TransactionBuilder<{ ok: null } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: null } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'HandleRequestBridgingInterruptedTransfer',
      msg_id,
      '[u8;32]',
      'Result<Null, Error>',
      this._program.programId,
    );
  }

  /**
   * The method is intended for tests and is available only when the feature `mocks`
   * is enabled. Inserts the message info into the corresponding collection.
   */
  public insertMessageInfo(_msg_id: MessageId, _status: MessageStatus, _details: TxDetails): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'InsertMessageInfo',
      [_msg_id, _status, _details],
      '([u8;32], MessageStatus, TxDetails)',
      'Null',
      this._program.programId,
    );
  }

  public insertTransactions(
    data: Array<[number | string | bigint, number | string | bigint]>,
  ): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'InsertTransactions',
      data,
      'Vec<(u64, u64)>',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Add a new token pair to a [State::token_map]. Can be called only by a [State::admin].
   */
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
      'VftManager',
      'MapVaraToEthAddress',
      [vara_token_id, eth_token_id, supply_type],
      '([u8;32], H160, TokenSupply)',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Remove the token pair from [State::token_map]. Can be called only by a [State::admin].
   */
  public removeVaraToEthAddress(vara_token_id: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'RemoveVaraToEthAddress',
      vara_token_id,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Request bridging of tokens from Gear to Ethereum.
   *
   * Allowance should be granted to the current program to spend `amount` tokens
   * from the source address.
   */
  public requestBridging(
    vara_token_id: ActorId,
    amount: number | string | bigint,
    receiver: H160,
  ): TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: [number | string | bigint, H160] } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'RequestBridging',
      [vara_token_id, amount, receiver],
      '([u8;32], U256, H160)',
      'Result<(U256, H160), Error>',
      this._program.programId,
    );
  }

  /**
   * Change [State::admin]. Can be called only by a [State::admin].
   */
  public setAdmin(new_admin: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'SetAdmin',
      new_admin,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Change [State::pause_admin]. Can be called only by a [State::admin].
   */
  public setPauseAdmin(new_pause_admin: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'SetPauseAdmin',
      new_pause_admin,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Submit rlp-encoded transaction receipt.
   *
   * This receipt is decoded under the hood and checked that it's a valid receipt from tx
   * sent to `ERC20Manager` contract.
   *
   * This method can be called only by [State::historical_proxy_address] program.
   */
  public submitReceipt(
    slot: number | string | bigint,
    transaction_index: number | string | bigint,
    receipt_rlp: `0x${string}`,
  ): TransactionBuilder<{ ok: null } | { err: Error }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: null } | { err: Error }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'SubmitReceipt',
      [slot, transaction_index, receipt_rlp],
      '(u64, u64, Vec<u8>)',
      'Result<Null, Error>',
      this._program.programId,
    );
  }

  /**
   * Change [State::erc20_manager_address]. Can be called only by a [State::admin].
   */
  public updateErc20ManagerAddress(erc20_manager_address_new: H160): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'UpdateErc20ManagerAddress',
      erc20_manager_address_new,
      'H160',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Change [State::historical_proxy_address]. Can be called only by a [State::admin].
   */
  public updateHistoricalProxyAddress(historical_proxy_address_new: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'UpdateHistoricalProxyAddress',
      historical_proxy_address_new,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  public updateVfts(vft_map: Array<[ActorId, ActorId]>): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'UpdateVfts',
      vft_map,
      'Vec<([u8;32], [u8;32])>',
      'Null',
      this._program.programId,
    );
  }

  public upgrade(vft_manager_new: ActorId): TransactionBuilder<null> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<null>(
      this._program.api,
      this._program.registry,
      'send_message',
      'VftManager',
      'Upgrade',
      vft_manager_new,
      '[u8;32]',
      'Null',
      this._program.programId,
    );
  }

  /**
   * Get current [State::admin] address.
   */
  public async admin(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'Admin']).toHex();
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

  /**
   * Get current [State::erc20_manager_address] address.
   */
  public async erc20ManagerAddress(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<H160 | null> {
    const payload = this._program.registry
      .createType('(String, String)', ['VftManager', 'Erc20ManagerAddress'])
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
    const result = this._program.registry.createType('(String, String, Option<H160>)', reply.payload);
    return result[2].toJSON() as unknown as H160 | null;
  }

  /**
   * Get current [State::gear_bridge_builtin] address.
   */
  public async gearBridgeBuiltin(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'GearBridgeBuiltin']).toHex();
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

  /**
   * Get current [Config].
   */
  public async getConfig(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Config> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'GetConfig']).toHex();
    const reply = await this._program.api.message.calculateReply({
      destination: this._program.programId,
      origin: originAddress ? decodeAddress(originAddress) : ZERO_ADDRESS,
      payload,
      value: value || 0,
      gasLimit: this._program.api.blockGasLimit.toBigInt(),
      at: atBlock,
    });
    throwOnErrorReply(reply.code, reply.payload.toU8a(), this._program.api.specVersion, this._program.registry);
    const result = this._program.registry.createType('(String, String, Config)', reply.payload);
    return result[2].toJSON() as unknown as Config;
  }

  /**
   * Get current [State::historical_proxy_address].
   */
  public async historicalProxyAddress(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry
      .createType('(String, String)', ['VftManager', 'HistoricalProxyAddress'])
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
    const result = this._program.registry.createType('(String, String, [u8;32])', reply.payload);
    return result[2].toJSON() as unknown as ActorId;
  }

  /**
   * Check if `vft-manager` is currently paused.
   */
  public async isPaused(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<boolean> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'IsPaused']).toHex();
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

  /**
   * Get current [State::pause_admin] address.
   */
  public async pauseAdmin(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<ActorId> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'PauseAdmin']).toHex();
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

  /**
   * Get state of a `request_bridging` message tracker.
   */
  public async requestBridingMsgTrackerState(
    start: number,
    count: number,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[MessageId, MessageInfo]>> {
    const payload = this._program.registry
      .createType('(String, String, u32, u32)', ['VftManager', 'RequestBridingMsgTrackerState', start, count])
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
    const result = this._program.registry.createType('(String, String, Vec<([u8;32], MessageInfo)>)', reply.payload);
    return result[2].toJSON() as unknown as Array<[MessageId, MessageInfo]>;
  }

  public async transactions(
    order: Order,
    start: number,
    count: number,
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[number | string | bigint, number | string | bigint]>> {
    const payload = this._program.registry
      .createType('(String, String, Order, u32, u32)', ['VftManager', 'Transactions', order, start, count])
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
    const result = this._program.registry.createType('(String, String, Vec<(u64, u64)>)', reply.payload);
    return result[2].toJSON() as unknown as Array<[number | string | bigint, number | string | bigint]>;
  }

  /**
   * Get current [token mapping](State::token_map).
   */
  public async varaToEthAddresses(
    originAddress?: string,
    value?: number | string | bigint,
    atBlock?: `0x${string}`,
  ): Promise<Array<[ActorId, H160, TokenSupply]>> {
    const payload = this._program.registry.createType('(String, String)', ['VftManager', 'VaraToEthAddresses']).toHex();
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
      '(String, String, Vec<([u8;32], H160, TokenSupply)>)',
      reply.payload,
    );
    return result[2].toJSON() as unknown as Array<[ActorId, H160, TokenSupply]>;
  }

  /**
   * Token mapping was added.
   *
   * This means that VFT Manager service now supports specified
   * [vara_token_id](Event::TokenMappingAdded::vara_token_id)/[eth_token_id](Event::TokenMappingAdded::eth_token_id) pair.
   */
  public subscribeToTokenMappingAddedEvent(
    callback: (data: { vara_token_id: ActorId; eth_token_id: H160; supply_type: TokenSupply }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'TokenMappingAdded') {
        callback(
          this._program.registry
            .createType(
              '(String, String, {"vara_token_id":"[u8;32]","eth_token_id":"H160","supply_type":"TokenSupply"})',
              message.payload,
            )[2]
            .toJSON() as unknown as { vara_token_id: ActorId; eth_token_id: H160; supply_type: TokenSupply },
        );
      }
    });
  }

  /**
   * Token mapping was removed.
   *
   * This means that VFT Manager service doesn't support specified
   * [vara_token_id](Event::TokenMappingRemoved::vara_token_id)/[eth_token_id](Event::TokenMappingRemoved::eth_token_id)
   * pair anymore.
   */
  public subscribeToTokenMappingRemovedEvent(
    callback: (data: { vara_token_id: ActorId; eth_token_id: H160; supply_type: TokenSupply }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'TokenMappingRemoved') {
        callback(
          this._program.registry
            .createType(
              '(String, String, {"vara_token_id":"[u8;32]","eth_token_id":"H160","supply_type":"TokenSupply"})',
              message.payload,
            )[2]
            .toJSON() as unknown as { vara_token_id: ActorId; eth_token_id: H160; supply_type: TokenSupply },
        );
      }
    });
  }

  /**
   * Bridging of tokens from Gear to Ethereum was requested.
   *
   * When this event is emitted it means that `VFT` tokens were locked/burned and
   * a message to the gear-eth-bridge built-in actor was successfully submitted.
   */
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

  /**
   * Vft-manager was paused by an admin.
   *
   * It means that any user requests to it will be rejected.
   */
  public subscribeToPausedEvent(callback: (data: null) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'Paused') {
        callback(null);
      }
    });
  }

  /**
   * Vft-manager was unpaused by an admin.
   *
   * It means that normal operation is continued after the pause.
   */
  public subscribeToUnpausedEvent(callback: (data: null) => void | Promise<void>): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'Unpaused') {
        callback(null);
      }
    });
  }

  /**
   * Address of the `historical-proxy` program was changed.
   */
  public subscribeToHistoricalProxyAddressChangedEvent(
    callback: (data: { old: ActorId; new: ActorId }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (
        getServiceNamePrefix(payload) === 'VftManager' &&
        getFnNamePrefix(payload) === 'HistoricalProxyAddressChanged'
      ) {
        callback(
          this._program.registry
            .createType('(String, String, {"old":"[u8;32]","new":"[u8;32]"})', message.payload)[2]
            .toJSON() as unknown as { old: ActorId; new: ActorId },
        );
      }
    });
  }

  /**
   * Address of the `ERC20Manager` contract address on Ethereum was changed.
   */
  public subscribeToErc20ManagerAddressChangedEvent(
    callback: (data: { old: H160; new: H160 }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'Erc20ManagerAddressChanged') {
        callback(
          this._program.registry
            .createType('(String, String, {"old":"H160","new":"H160"})', message.payload)[2]
            .toJSON() as unknown as { old: H160; new: H160 },
        );
      }
    });
  }

  /**
   * Transaction receipt submitted via [VftManager::submit_receipt] processed successfully.
   */
  public subscribeToBridgingAcceptedEvent(
    callback: (data: {
      to: ActorId;
      from: H160;
      amount: number | string | bigint;
      token: ActorId;
    }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'VftManager' && getFnNamePrefix(payload) === 'BridgingAccepted') {
        callback(
          this._program.registry
            .createType(
              '(String, String, {"to":"[u8;32]","from":"H160","amount":"U256","token":"[u8;32]"})',
              message.payload,
            )[2]
            .toJSON() as unknown as { to: ActorId; from: H160; amount: number | string | bigint; token: ActorId },
        );
      }
    });
  }
}
