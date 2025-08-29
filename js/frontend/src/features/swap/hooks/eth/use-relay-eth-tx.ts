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

  const relay = async (onLog: (message: string) => void) => {
    definedAssert(api, 'API');
    definedAssert(account, 'Account');
    definedAssert(publicClient, 'Ethereum Public Client');

    const { error, ok, ...result } = await relayEthToVara(
      txHash,
      ETH_BEACON_NODE_ADDRESS,
      publicClient,
      api,
      CONTRACT_ADDRESS.CHECKPOINT_CLIENT,
      CONTRACT_ADDRESS.HISTORICAL_PROXY,
      CONTRACT_ADDRESS.VFT_MANAGER,
      'VftManager',
      'SubmitReceipt',
      account.decodedAddress,
      { signer: account.signer },
      onLog,
    );

    if (error) throw new Error(JSON.stringify(error));
    if (!ok) throw new Error('Failed to relay Ethereum transaction');

    return result;
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayEthTx };
