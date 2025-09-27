import { GearApi, HexString } from '@gear-js/api';
import { relayEthToVara } from '@gear-js/bridge';
import { useAccount } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient } from 'wagmi';

import { definedAssert } from '@/utils';

import { ETH_BEACON_NODE_ADDRESS, CONTRACT_ADDRESS } from '../../consts';

type Params = {
  api: GearApi;
  onLog: (message: string) => void;
};

function useRelayEthTx(txHash: HexString) {
  const { account } = useAccount();

  const publicClient = usePublicClient();

  const relay = async ({ api, onLog }: Params) => {
    definedAssert(account, 'Account');
    definedAssert(publicClient, 'Ethereum Public Client');

    const { error, ok, ...result } = await relayEthToVara({
      transactionHash: txHash,
      beaconRpcUrl: ETH_BEACON_NODE_ADDRESS,
      ethereumPublicClient: publicClient,
      gearApi: api,
      checkpointClientId: CONTRACT_ADDRESS.CHECKPOINT_CLIENT,
      historicalProxyId: CONTRACT_ADDRESS.HISTORICAL_PROXY,
      clientId: CONTRACT_ADDRESS.VFT_MANAGER,
      clientServiceName: 'VftManager',
      clientMethodName: 'SubmitReceipt',
      signer: account.decodedAddress,
      signerOptions: { signer: account.signer },
      statusCb: onLog,
    });

    if (error) throw new Error(JSON.stringify(error));
    if (!ok) throw new Error('Failed to relay Ethereum transaction');

    return result;
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayEthTx };
