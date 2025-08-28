import { HexString } from '@gear-js/api';
import { relayEthToVara } from '@gear-js/bridge';
import { useApi, useAccount } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient } from 'wagmi';

import { definedAssert } from '@/utils';

import { ETH_BEACON_NODE_ADDRESS, CONTRACT_ADDRESS } from '../../consts';

function useRelayEthTx(txHash: HexString) {
  const { api } = useApi();
  const { account } = useAccount();

  const publicClient = usePublicClient();

  const relay = () => {
    definedAssert(api, 'API');
    definedAssert(account, 'Account');
    definedAssert(publicClient, 'Ethereum Public Client');

    const clientId = '0x00';

    return relayEthToVara(
      txHash,
      ETH_BEACON_NODE_ADDRESS,
      publicClient,
      api,
      CONTRACT_ADDRESS.CHECKPOINT_CLIENT,
      CONTRACT_ADDRESS.HISTORICAL_PROXY,
      clientId,
      'serviceName',
      'methodName',
      account.decodedAddress,
      { signer: account.signer },
      false,
    );
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayEthTx };
