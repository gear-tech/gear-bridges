import { GearApi, BaseGearProgram } from '@gear-js/api';
import { TypeRegistry } from '@polkadot/types';
import {
  TransactionBuilder,
  ActorId,
  getServiceNamePrefix,
  getFnNamePrefix,
  ZERO_ADDRESS,
  QueryBuilder,
} from 'sails-js';

export type ProxyError =
  /**
   * Endpoint for requested slot not found.
   */
  | { NoEndpointForSlot: number | string | bigint }
  /**
   * Failed to send message.
   */
  | { SendFailure: string }
  /**
   * Failed to receive reply.
   */
  | { ReplyFailure: string }
  /**
   * Failed to decode reply.
   */
  | { DecodeFailure: string }
  /**
   * `eth-events-*` returned error.
   */
  | { EthereumEventClient: Error };

export class HistoricalProxyClient {
  public readonly registry: TypeRegistry;
  public readonly historicalProxy: HistoricalProxy;
  private _program: BaseGearProgram;

  constructor(
    public api: GearApi,
    programId: `0x${string}`,
  ) {
    const types: Record<string, any> = {
      ProxyError: {
        _enum: {
          NoEndpointForSlot: 'u64',
          SendFailure: 'String',
          ReplyFailure: 'String',
          DecodeFailure: 'String',
          EthereumEventClient: 'Error',
        },
      },
      Error: {
        _enum: [
          'DecodeReceiptEnvelopeFailure',
          'FailedEthTransaction',
          'SendFailure',
          'ReplyFailure',
          'HandleResultDecodeFailure',
          'MissingCheckpoint',
          'InvalidBlockProof',
          'TrieDbFailure',
          'InvalidReceiptProof',
        ],
      },
    };

    this.registry = new TypeRegistry();
    this.registry.setKnownTypes({ types });
    this.registry.register(types);
    this._program = new BaseGearProgram(programId, api);

    this.historicalProxy = new HistoricalProxy(this);
  }

  public get programId(): `0x${string}` {
    if (!this._program) throw new Error(`Program ID is not set`);
    return this._program.id;
  }
}

export class HistoricalProxy {
  constructor(private _program: HistoricalProxyClient) {}

  /**
   * Redirect message to `eth-events-*` program which is valid for `slot`.
   * If message is relayed successfully then reply is sent to `client` address
   * to `client_route` route.
   *
   * # Parameters
   *
   * - `slot`: slot for which message is relayed.
   * - `proofs`: SCALE encoded `EthToVaraEvent`.
   * - `client`: client address to send receipt to on success.
   * - `client_route`: route to send receipt to on success.
   *
   * # Returns
   *
   * - `(Vec<u8>, Vec<u8>)`: on success where first vector is receipt and second vector is reply from calling `client_route`.
   * - `ProxyError`: if redirect failed
   */
  public redirect(
    slot: number | string | bigint,
    proofs: `0x${string}`,
    client: ActorId,
    client_route: `0x${string}`,
  ): TransactionBuilder<{ ok: [`0x${string}`, `0x${string}`] } | { err: ProxyError }> {
    if (!this._program.programId) throw new Error('Program ID is not set');
    return new TransactionBuilder<{ ok: [`0x${string}`, `0x${string}`] } | { err: ProxyError }>(
      this._program.api,
      this._program.registry,
      'send_message',
      'HistoricalProxy',
      'Redirect',
      [slot, proofs, client, client_route],
      '(u64, Vec<u8>, [u8;32], Vec<u8>)',
      'Result<(Vec<u8>, Vec<u8>), ProxyError>',
      this._program.programId,
    );
  }

  /**
   * Get endpoint for the specified `slot`.
   */
  public endpointFor(slot: number): QueryBuilder<{ ok: ActorId } | { err: ProxyError }> {
    return new QueryBuilder<{ ok: ActorId } | { err: ProxyError }>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'HistoricalProxy',
      'EndpointFor',
      slot,
      'u64',
      'Result<[u8;32], ProxyError>',
    );
  }

  /**
   * Get endpoint map stored in this service.
   */
  public endpoints(): QueryBuilder<Array<[number | string | bigint, ActorId]>> {
    return new QueryBuilder<Array<[number | string | bigint, ActorId]>>(
      this._program.api,
      this._program.registry,
      this._program.programId,
      'HistoricalProxy',
      'Endpoints',
      null,
      null,
      'Vec<(u64, [u8;32])>',
    );
  }

  /**
   * Tx receipt is checked to be valid and successfully sent to the
   * underlying program.
   */
  public subscribeToRelayedEvent(
    callback: (data: {
      slot: number | string | bigint;
      block_number: number | string | bigint;
      transaction_index: number;
    }) => void | Promise<void>,
  ): Promise<() => void> {
    return this._program.api.gearEvents.subscribeToGearEvent('UserMessageSent', ({ data: { message } }) => {
      if (!message.source.eq(this._program.programId) || !message.destination.eq(ZERO_ADDRESS)) {
        return;
      }

      const payload = message.payload.toHex();
      if (getServiceNamePrefix(payload) === 'HistoricalProxy' && getFnNamePrefix(payload) === 'Relayed') {
        callback(
          this._program.registry
            .createType(
              '(String, String, {"slot":"u64","block_number":"u64","transaction_index":"u32"})',
              message.payload,
            )[2]
            .toJSON() as unknown as {
            slot: number | string | bigint;
            block_number: number | string | bigint;
            transaction_index: number;
          },
        );
      }
    });
  }
}
