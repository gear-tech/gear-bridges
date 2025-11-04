import { KeyringPair } from '@polkadot/keyring/types';
import { SignerOptions } from '@polkadot/api/types';
import { GearApi } from '@gear-js/api';
import { PublicClient } from 'viem';

import { encodeEthToVaraEvent, getPrefix, HistoricalProxyClient, ProxyError } from '../vara/index.js';
import { createBeaconClient, createEthereumClient } from '../ethereum/index.js';
import { composeProof } from './proof-composer.js';
import { StatusCb } from '../util.js';

interface RelayResult {
  blockHash: string;
  msgId: string;
  txHash: string;
  isFinalized: Promise<boolean>;
  ok?: string;
  error?: ProxyError;
}

/**
 * Parameters for relaying an Ethereum transaction to the Vara network.
 * This interface defines all the required configuration and optional settings
 * needed to relay cross-chain transactions from Ethereum to Vara.
 */
export type RelayEthToVaraParams = {
  /**
   * Transaction hash of the Ethereum transaction to relay
   */
  transactionHash: `0x${string}`;
  /**
   * The RPC URL for the Ethereum beacon chain client
   */
  beaconRpcUrl: string;
  /**
   * Viem public client for Ethereum network interactions
   */
  ethereumPublicClient: PublicClient;
  /**
   * Gear API instance for Vara network operations
   */
  gearApi: GearApi;
  /**
   * ID of the historical proxy program on Vara
   */
  historicalProxyId: `0x${string}`;
  /**
   * ID of the target client program on Vara
   */
  clientId: `0x${string}`;
  /**
   * Name of the service to call on the target client
   */
  clientServiceName: string;
  /**
   * Name of the method to call on the target service
   */
  clientMethodName: string;
  /**
   * Flag indicating whether to wait for the slot to appear on the CheckpointClient contract
   */
  wait?: boolean;
  /**
   * Account signer, either as string address or KeyringPair for transaction signing
   */
  signer: string | KeyringPair;
  /**
   * Optional signing configuration parameters
   */
  signerOptions?: Partial<SignerOptions>;
  /**
   * Callback function to track the status of the transaction
   */
  statusCb?: StatusCb;
};

/**
 * Relays an Ethereum transaction to the Vara network through the bridge infrastructure.
 *
 * This function performs the complete relay process:
 * 1. Creates beacon and Ethereum clients for proof generation
 * 2. Composes cryptographic proof of the Ethereum transaction
 * 3. Builds and submits a transaction to the Vara network
 * 4. Waits for the transaction response and returns the result
 *
 * @param params - Configuration parameters for the relay operation
 * @param params.transactionHash - Hash of the Ethereum transaction to relay
 * @param params.beaconRpcUrl - RPC URL for the Ethereum beacon chain client
 * @param params.ethereumPublicClient - Viem public client for Ethereum interactions
 * @param params.gearApi - Gear API instance for Vara network operations
 * @param params.checkpointClientId - ID of the checkpoint client program on Vara
 * @param params.historicalProxyId - ID of the historical proxy program on Vara
 * @param params.clientId - ID of the target client program on Vara
 * @param params.clientServiceName - Name of the service to call on the target client
 * @param params.clientMethodName - Name of the method to call on the target service
 * @param params.wait - If true, waits for the slot to appear on CheckpointClient instead of returning error
 * @param params.signer - Account signer for transaction signing (address string or KeyringPair)
 * @param params.signerOptions - Optional signing configuration parameters
 * @param params.statusCb - Optional callback function to track transaction status
 *
 * @returns Promise resolving to relay result with transaction details and either success data or error
 *
 * @throws {Error} When beacon client creation fails
 * @throws {Error} When proof composition fails
 * @throws {Error} When transaction building or submission fails
 *
 * @example
 * ```typescript
 * const result = await relayEthToVara({
 *   transactionHash: '0x123...',
 *   beaconRpcUrl: 'https://beacon-chain.example.com',
 *   ethereumPublicClient: viemClient,
 *   gearApi: api,
 *   checkpointClientId: '0xabc...',
 *   historicalProxyId: '0xdef...',
 *   clientId: '0x456...',
 *   clientServiceName: 'BridgeService',
 *   clientMethodName: 'processTransfer',
 *   signer: keyringPair,
 *   statusCb: (status, details) => console.log(status, details)
 * });
 *
 * if ('error' in result) {
 *   console.error('Relay failed:', result.error);
 * } else {
 *   console.log('Relay succeeded:', result.ok);
 * }
 * ```
 */
export async function relayEthToVara(params: RelayEthToVaraParams): Promise<RelayResult> {
  const wait = params.wait ?? false;
  const statusCb = params.statusCb || (() => {});

  const beaconClient = await createBeaconClient(params.beaconRpcUrl);
  const ethClient = createEthereumClient(params.ethereumPublicClient, beaconClient);

  const historicalProxyClient = new HistoricalProxyClient(params.gearApi, params.historicalProxyId);

  statusCb(`Composing proof`, { txHash: params.transactionHash });
  const composeResult = await composeProof(
    beaconClient,
    ethClient,
    historicalProxyClient,
    params.transactionHash,
    wait,
    statusCb,
  );

  statusCb(`Building transaction`, { slot: composeResult.proofBlock.block.slot.toString() });
  const encodedEthToVaraEvent = encodeEthToVaraEvent(composeResult);

  const tx = historicalProxyClient.historicalProxy
    .redirect(
      composeResult.proofBlock.block.slot,
      encodedEthToVaraEvent,
      params.clientId,
      getPrefix(params.clientServiceName, params.clientMethodName),
    )
    .withAccount(params.signer, params.signerOptions)
    .withGas('max');

  statusCb(`Sending transaction`, { historicalProxyId: params.historicalProxyId });
  const { blockHash, msgId, txHash, response, isFinalized } = await tx.signAndSend();

  statusCb(`Waiting for response`, { txHash, blockHash });
  const reply = await response();

  const txDetails = {
    blockHash,
    msgId,
    txHash,
    isFinalized,
  };

  if ('err' in reply) {
    return {
      ...txDetails,
      error: reply.err,
    };
  } else {
    return {
      ...txDetails,
      ok: reply.ok[1],
    };
  }
}
