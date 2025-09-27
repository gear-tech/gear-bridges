import { GearApi, HexString } from '@gear-js/api';
import { relayVaraToEth } from '@gear-js/bridge';
import { useMutation } from '@tanstack/react-query';
import { useConfig, usePublicClient, useWalletClient } from 'wagmi';
import { waitForTransactionReceipt } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';

type Params = {
  api: GearApi;
  onLog: (message: string) => void;
};

function useRelayVaraTx(nonce: HexString, blockNumber: bigint) {
  const publicClient = usePublicClient();
  const { data: walletClient } = useWalletClient();
  const config = useConfig();

  const relay = async ({ api, onLog }: Params) => {
    definedAssert(publicClient, 'Ethereum Public Client');
    definedAssert(walletClient, 'Wallet Client');

    const { error, success, ...result } = await relayVaraToEth({
      nonce,
      blockNumber,
      ethereumPublicClient: publicClient,
      ethereumWalletClient: walletClient,
      ethereumAccount: walletClient.account,
      gearApi: api,
      messageQueueAddress: CONTRACT_ADDRESS.ETH_MESSAGE_QUEUE,
      statusCb: onLog,
    });

    if (error) throw new Error(error);
    if (!success) throw new Error('Failed to relay Vara transaction');

    const isExtraConfirmed = waitForTransactionReceipt(config, {
      hash: result.transactionHash,
      confirmations: 2,
    });

    return { ...result, isExtraConfirmed };
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayVaraTx };
