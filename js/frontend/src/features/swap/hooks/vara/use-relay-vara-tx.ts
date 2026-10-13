import { GearApi } from '@gear-js/api';
import { relayVaraToEth, type OutboundEffect } from '@gear-js/bridge';
import { useMutation } from '@tanstack/react-query';
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

function useRelayVaraTx(nonce: bigint, blockNumber: bigint, expectedEffect: OutboundEffect) {
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

      const { error, success, transactionHash } = await relayVaraToEth({
        nonce,
        blockNumber,
        ethereumPublicClient: publicClient,
        ethereumWalletClient: walletClient,
        ethereumAccount: walletClient.account,
        gearApi: archiveApi,
        messageQueueAddress: NETWORK_PRESET.ETH_MESSAGE_QUEUE_CONTRACT_ADDRESS,
        expectedEffect,
        statusCb: onLog,
        wait: true,
        deadline,
      });

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
