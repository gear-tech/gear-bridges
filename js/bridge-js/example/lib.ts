// Generated ABI from ping/ping.idl; constructors use an explicit journaled salt.
import { generateCodeHash, generateProgramId } from '@gear-js/api';
import type { GearApi, HexString, IProgramUploadResult } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import type { ITuple } from '@polkadot/types/types';
import { u8aEq, u8aToHex, u8aToU8a } from '@polkadot/util';
import {
  TransactionBuilder,
  QueryBuilder,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
} from 'sails-js';
import type { ActorId, H160, H256 } from 'sails-js';

export interface BridgeConfig {
  builtin: ActorId;
  fee_bridge: number | string | bigint;
  gas_to_send_request_to_builtin: number | string | bigint;
  gas_for_reply_deposit: number | string | bigint;
  reply_timeout: number;
}

export interface QueuedDelivery {
  block_number: number;
  hash: H256;
  nonce: number | string | bigint;
  queue_id: number | string | bigint;
}

export type PingError =
  | 'NotHistoricalProxy'
  | 'NotOwner'
  | 'InvalidReceipt'
  | 'UnsupportedEvent'
  | 'WrongSender'
  | 'WrongDestination'
  | 'InvalidPayload'
  | 'AmbiguousDelivery'
  | 'AlreadyProcessed'
  | 'AlreadyReceived'
  | 'AlreadyRequested'
  | 'BuiltinFailure'
  | 'BuiltinDecode';

export interface Delivery {
  application_id: H256;
  payload: HexString;
}

export type OutboundStatus = { pending: null } | { queued: QueuedDelivery };
export type PingResult<T> = { ok: T } | { err: PingError };
type ReplyResults = { SubmitReceipt: PingResult<Delivery>; SendMessage: PingResult<QueuedDelivery> };
export type PreparedPingUpload = Pick<IProgramUploadResult, 'programId' | 'codeId' | 'extrinsic'>;

export class PingClient {
  public readonly registry: TypeRegistry;
  public readonly ping: Ping;

  constructor(
    public api: GearApi,
    private _programId?: HexString,
  ) {
    const types = {
      BridgeConfig: {
        builtin: '[u8;32]',
        fee_bridge: 'u128',
        gas_to_send_request_to_builtin: 'u64',
        gas_for_reply_deposit: 'u64',
        reply_timeout: 'u32',
      },
      QueuedDelivery: { block_number: 'u32', hash: 'H256', nonce: 'U256', queue_id: 'u64' },
      PingError: {
        _enum: [
          'NotHistoricalProxy', 'NotOwner', 'InvalidReceipt', 'UnsupportedEvent', 'WrongSender',
          'WrongDestination', 'InvalidPayload', 'AmbiguousDelivery', 'AlreadyProcessed', 'AlreadyReceived',
          'AlreadyRequested', 'BuiltinFailure', 'BuiltinDecode',
        ],
      },
      Delivery: { application_id: 'H256', payload: 'Vec<u8>' },
      OutboundStatus: { _enum: { Pending: 'Null', Queued: 'QueuedDelivery' } },
    };
    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    this.ping = new Ping(this);
  }

  public get programId(): HexString {
    if (!this._programId) throw new Error('Program ID is not set');
    return this._programId;
  }

  newCtorFromCode(
    code: Uint8Array | HexString,
    salt: Uint8Array | HexString,
    historical_proxy: ActorId,
    ethereum_emitter: H160,
    ethereum_sender: H160,
    ethereum_receiver: H160,
    bridge_config: BridgeConfig,
    gasLimit: bigint,
  ): PreparedPingUpload {
    const saltBytes = u8aToU8a(salt);
    if (saltBytes.length !== 32) throw new Error('Ping requires its recorded 32-byte salt');
    const codeId = generateCodeHash(code);
    const programId = generateProgramId(codeId, saltBytes);
    const initPayload = this.registry.createType(
      '(String, [u8;32], H160, H160, H160, BridgeConfig)',
      ['New', historical_proxy, ethereum_emitter, ethereum_sender, ethereum_receiver, bridge_config],
    ).toHex();
    const prepared = this.api.program.upload({ code, salt: u8aToHex(saltBytes), initPayload, gasLimit, value: 0 });
    if (prepared.codeId !== codeId || prepared.programId !== programId) {
      throw new Error('Prepared Ping upload disagrees with the recorded code/salt prediction');
    }
    return { programId, codeId, extrinsic: prepared.extrinsic };
  }

  decodeReply<Method extends keyof ReplyResults>(method: Method, payload: Uint8Array | HexString): ReplyResults[Method] {
    const resultType = { SubmitReceipt: 'Delivery', SendMessage: 'QueuedDelivery' }[method];
    if (!resultType) throw new Error('Unsupported Ping reply method');
    const bytes = u8aToU8a(payload);
    const decoded = this.registry.createType<ITuple>(`(String, String, Result<${resultType}, PingError>)`, bytes);
    if (!u8aEq(decoded.toU8a(), bytes) || decoded[0].toString() !== 'Ping' || decoded[1].toString() !== method) {
      throw new Error('Malformed, trailing or wrong-route Ping reply');
    }
    return decoded[2].toJSON() as ReplyResults[Method];
  }
}

export class Ping {
  constructor(private _program: PingClient) {}

  public sendMessage(application_id: H256, payload: HexString): TransactionBuilder<PingResult<QueuedDelivery>> {
    return new TransactionBuilder<PingResult<QueuedDelivery>>(
      this._program.api, this._program.registry, 'send_message', 'Ping', 'SendMessage',
      [application_id, payload], '(H256, Vec<u8>)', 'Result<QueuedDelivery, PingError>', this._program.programId,
    );
  }

  public submitReceipt(
    slot: number | string | bigint,
    transaction_index: number | string | bigint,
    receipt_rlp: HexString,
  ): TransactionBuilder<PingResult<Delivery>> {
    return new TransactionBuilder<PingResult<Delivery>>(
      this._program.api, this._program.registry, 'send_message', 'Ping', 'SubmitReceipt',
      [slot, transaction_index, receipt_rlp], '(u64, u64, Vec<u8>)', 'Result<Delivery, PingError>', this._program.programId,
    );
  }

  public outbound(application_id: H256): QueryBuilder<OutboundStatus | null> {
    return new QueryBuilder<OutboundStatus | null>(
      this._program.api, this._program.registry, this._program.programId,
      'Ping', 'Outbound', application_id, 'H256', 'Option<OutboundStatus>',
    );
  }

  public payloadOf(application_id: H256): QueryBuilder<HexString | null> {
    return new QueryBuilder<HexString | null>(
      this._program.api, this._program.registry, this._program.programId,
      'Ping', 'PayloadOf', application_id, 'H256', 'Option<Vec<u8>>',
    );
  }

  public received(slot: number | string | bigint, transaction_index: number | string | bigint): QueryBuilder<Delivery | null> {
    return new QueryBuilder<Delivery | null>(
      this._program.api, this._program.registry, this._program.programId,
      'Ping', 'Received', [slot, transaction_index], '(u64, u64)', 'Option<Delivery>',
    );
  }

  public subscribeToReceiptSubmittedEvent(
    callback: (data: { slot: number | string | bigint; transaction_index: number | string | bigint; application_id: H256; payload: HexString }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) return;
      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) !== 'Ping' || getFnNamePrefix(payload) !== 'ReceiptSubmitted') return;
      const decoded = this._program.registry.createType(
        '(String, String, {"slot":"u64","transaction_index":"u64","application_id":"H256","payload":"Vec<u8>"})',
        message.payload,
      );
      if (!u8aEq(decoded.toU8a(), message.payload.toU8a(true))) throw new Error('Malformed Ping receipt event');
      void callback(decoded[2].toJSON() as unknown as Parameters<typeof callback>[0]);
    });
  }
}
