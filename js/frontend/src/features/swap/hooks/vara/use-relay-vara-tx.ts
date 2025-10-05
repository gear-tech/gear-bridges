import { relayVaraToEth } from '@gear-js/bridge';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient, useWalletClient } from 'wagmi';

import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';
import { initArchiveApi } from '../../utils';

type Params = {
  onLog: (message: string) => void;
  onReceipt: () => void;
  onError: (error: Error) => void;
};

function useRelayVaraTx(nonce: bigint, blockNumber: bigint) {
  const publicClient = usePublicClient();
  const { data: walletClient } = useWalletClient();

  const relay = async ({ onLog, onReceipt, onError }: Params) => {
    definedAssert(publicClient, 'Ethereum Public Client');
    definedAssert(walletClient, 'Wallet Client');

    const archiveApi = await initArchiveApi();

    try {
      const { error, success } = await relayVaraToEth({
        nonce,
        blockNumber,
        ethereumPublicClient: publicClient,
        ethereumWalletClient: walletClient,
        ethereumAccount: walletClient.account,
        gearApi: archiveApi,
        messageQueueAddress: CONTRACT_ADDRESS.ETH_MESSAGE_QUEUE,
        statusCb: onLog,
      });

      if (error) throw new Error(error);
      if (!success) throw new Error('Failed to relay Vara transaction');

      onReceipt();
    } catch (error) {
      onError(error as Error);
    } finally {
      await archiveApi.disconnect();
    }
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayVaraTx };
