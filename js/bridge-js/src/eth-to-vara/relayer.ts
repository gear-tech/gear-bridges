import { KeyringPair } from '@polkadot/keyring/types';
import { SignerOptions } from '@polkadot/api/types';
import { GearApi } from '@gear-js/api';
import { PublicClient } from 'viem';

import { CheckpointClient, encodeEthToVaraEvent, getPrefix, HistoricalProxyClient, ProxyError } from '../vara';
import { createBeaconClient, createEthereumClient } from '../ethereum';
import { composeProof } from './proof-composer';
import { initLogger, logger } from '../util';

interface RelayResult {
  blockHash: string;
  msgId: string;
  txHash: string;
  isFinalized: Promise<boolean>;
  ok?: string;
  error?: ProxyError;
}

/**
 * Relays an Ethereum transaction to the Vara network by creating a proof
 * and submitting it through historical proxy program.
 *
 * @param transactionHash - Transaction hash of the Ethereum transaction to relay
 * @param beaconRpcUrl - The RPC URL for the Ethereum beacon chain client
 * @param ethereumPublicClient - Viem public client for Ethereum network interactions
 * @param gearApi - Gear API instance for Vara network operations
 * @param checkpointClientId - ID of the checkpoint client program on Vara
 * @param historicalProxyId - ID of the historical proxy program on Vara
 * @param clientId - ID of the target client program on Vara
 * @param clientServiceName - Name of the service to call on the target client
 * @param clientMethodName - Name of the method to call on the target service
 * @param signer - Account signer, either as string address or KeyringPair for transaction signing
 * @param signerOptions - Optional signing configuration parameters
 * @returns Promise resolving to transaction details with either success data or error information
 */
export async function relayEthToVara(
  transactionHash: `0x${string}`,
  beaconRpcUrl: string,
  ethereumPublicClient: PublicClient,
  gearApi: GearApi,
  checkpointClientId: `0x${string}`,
  historicalProxyId: `0x${string}`,
  clientId: `0x${string}`,
  clientServiceName: string,
  clientMethodName: string,
  signer: string | KeyringPair,
  signerOptions?: Partial<SignerOptions>,
  silent = true,
): Promise<RelayResult> {
  initLogger(silent);
  const beaconClient = await createBeaconClient(beaconRpcUrl);
  const ethClient = createEthereumClient(ethereumPublicClient, beaconClient);

  const checkpointClient = new CheckpointClient(gearApi, checkpointClientId);

  const composeResult = await composeProof(beaconClient, ethClient, checkpointClient, transactionHash);

  logger.info(`Building transaction to be sent to Historical Proxy program`);
  const encodedEthToVaraEvent = encodeEthToVaraEvent(composeResult);

  const historicalProxyClient = new HistoricalProxyClient(gearApi, historicalProxyId);

  const tx = await historicalProxyClient.historicalProxy
    .redirect(
      composeResult.proofBlock.block.slot,
      encodedEthToVaraEvent,
      clientId,
      getPrefix(clientServiceName, clientMethodName),
    )
    .withAccount(signer, signerOptions)
    .calculateGas();

  logger.info(`Sending transaction`);
  const { blockHash, msgId, txHash, response, isFinalized } = await tx.signAndSend();

  logger.info(`Transaction sent with hash ${txHash} in block ${blockHash}`);
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
