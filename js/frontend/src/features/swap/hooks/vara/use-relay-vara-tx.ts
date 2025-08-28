import { HexString } from '@gear-js/api';
import { relayVaraToEth } from '@gear-js/bridge';
import { useApi, useAccount } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient, useWalletClient } from 'wagmi';

import { definedAssert } from '@/utils';

function useRelayVaraTx(nonce: bigint, blockNumber: bigint, messageQueuedAddress: HexString) {
  const { api } = useApi();
  const { account } = useAccount();
  const publicClient = usePublicClient();
  const { data: walletClient } = useWalletClient();

  const relay = () => {
    definedAssert(api, 'API');
    definedAssert(account, 'Account');
    definedAssert(publicClient, 'Ethereum Public Client');
    definedAssert(walletClient, 'Wallet Client');

    return relayVaraToEth(
      nonce,
      blockNumber,
      publicClient,
      walletClient,
      walletClient.account,
      api,
      messageQueuedAddress,
      false,
    );
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayVaraTx };
