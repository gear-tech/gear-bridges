import { GearApi } from '@gear-js/api';
import { prepareVaraToEthRelay, getMessageQueueClient, type OutboundEffect } from '@gear-js/bridge';
import { useMutation } from '@tanstack/react-query';
import { bytesToHex } from 'viem';
import { useConfig, usePublicClient } from 'wagmi';
import { getWalletClient } from 'wagmi/actions';

import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { useInitArchiveApi } from './use-init-archive-api';

type Params = {
  onLog: (message: string) => void;
  onFinalized: () => void;
  onError: (error: Error) => void;
};

function useRelayVaraTx(
  nonce: bigint,
  blockNumber: bigint,
  expectedEffect: Omit<Extract<OutboundEffect, { kind: 'token' }>, 'sourceActorId'>,
) {
  const { NETWORK_PRESET, syncEthWalletNetwork } = useNetworkType();
  const config = useConfig();
  const publicClient = usePublicClient();
  const initArchiveApi = useInitArchiveApi();

  const relay = async ({ onLog, onFinalized, onError }: Params) => {
    const deadline = Date.now() + 44 * 60 * 1000;
    let archiveApi: GearApi | undefined;

    try {
      definedAssert(publicClient, 'Ethereum Public Client');

      await syncEthWalletNetwork();

      // fetching wallet client after wallet network sync, otherwise it throws network mismatch error
      const walletClient = await getWalletClient(config);
      archiveApi = await initArchiveApi();

      const profile = NETWORK_PRESET.INBOUND_PROOF_PROFILE.profile;
      const deployment = profile?.queueDeployment;
      if (profile && deployment) {
        const [chainId, genesis] = await Promise.all([
          publicClient.getChainId(),
          publicClient.getBlock({ blockNumber: 0n }),
        ]);
        if (
          BigInt(chainId) !== profile.ethereumChainId ||
          genesis.hash.toLowerCase() !== profile.ethereumGenesisHash ||
          deployment.address !== NETWORK_PRESET.ETH_MESSAGE_QUEUE_CONTRACT_ADDRESS.toLowerCase()
        )
          throw new Error('HOLD: queue deployment approval belongs to another lane');
      }
      const prepared = await prepareVaraToEthRelay({
        messageQueueDeploymentBlock: deployment?.block,
        nonce,
        blockNumber,
        ethereumPublicClient: publicClient,
        gearApi: archiveApi,
        messageQueueAddress: NETWORK_PRESET.ETH_MESSAGE_QUEUE_CONTRACT_ADDRESS,
        statusCb: onLog,
        wait: true,
        deadline,
      });
      const queue = getMessageQueueClient(
        NETWORK_PRESET.ETH_MESSAGE_QUEUE_CONTRACT_ADDRESS,
        publicClient,
        walletClient,
        walletClient.account,
      );
      const { error, success, transactionHash } = await queue.processMessage(
        prepared.blockNumber,
        prepared.message,
        prepared.proof,
        { ...expectedEffect, sourceActorId: bytesToHex(prepared.message.source) },
        onLog,
        deadline,
        { wait: true },
      );

      if (!success)
        throw new Error(
          `HOLD: ${transactionHash !== '0x' ? `original Ethereum transaction ${transactionHash}; ` : ''}${error ?? 'Original delivery is unproven'}. Reconcile before retrying.`,
        );

      onFinalized();
    } catch (error) {
      onError(error as Error);
    } finally {
      await archiveApi?.disconnect();
    }
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayVaraTx };
