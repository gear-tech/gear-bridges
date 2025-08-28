import { HexString } from '@gear-js/api';
import { relayVaraToEth } from '@gear-js/bridge';
import { useApi } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient, useWalletClient } from 'wagmi';

import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';

function useRelayVaraTx(nonce: bigint | HexString, blockNumber: bigint) {
  const { api } = useApi();
  const publicClient = usePublicClient();
  const { data: walletClient } = useWalletClient();

  const relay = () => {
    definedAssert(api, 'API');
    definedAssert(publicClient, 'Ethereum Public Client');
    definedAssert(walletClient, 'Wallet Client');

    return relayVaraToEth(
      nonce,
      blockNumber,
      publicClient,
      walletClient,
      walletClient.account,
      api,
      CONTRACT_ADDRESS.ETH_MESSAGE_QUEUE,
      false,
    );
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayVaraTx };
