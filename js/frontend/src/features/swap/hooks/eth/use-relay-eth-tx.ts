import { GearApi, HexString } from '@gear-js/api';
import { relayEthToVara } from '@gear-js/bridge';
import { useAccount } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';
import { usePublicClient } from 'wagmi';

import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { useHistoricalProxyContractAddress, useInitArchiveApi } from '../vara';

type Params = {
  onLog: (message: string) => void;
  onInBlock: () => void;
  onError: (error: Error) => void;
};

function useRelayEthTx(txHash: HexString) {
  const { NETWORK_PRESET } = useNetworkType();
  const { account } = useAccount();
  const publicClient = usePublicClient();
  const { data: historicalProxyContractAddress } = useHistoricalProxyContractAddress();
  const initArchiveApi = useInitArchiveApi();

  const relay = async ({ onLog, onInBlock, onError }: Params) => {
    let archiveApi: GearApi | undefined;

    try {
      definedAssert(account, 'Account');
      definedAssert(publicClient, 'Ethereum Public Client');
      definedAssert(historicalProxyContractAddress, 'Historical Proxy Contract Address');

      archiveApi = await initArchiveApi();

      const { error, ok } = await relayEthToVara({
        transactionHash: txHash,
        beaconRpcUrl: NETWORK_PRESET.ETH_BEACON_NODE_ADDRESS,
        ethereumPublicClient: publicClient,
        gearApi: archiveApi,
        historicalProxyId: historicalProxyContractAddress,
        clientId: NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS,
        clientServiceName: 'VftManager',
        clientMethodName: 'SubmitReceipt',
        signer: account.decodedAddress,
        signerOptions: { signer: account.signer },
        statusCb: onLog,
      });

      if (error) throw new Error(JSON.stringify(error));
      if (!ok) throw new Error('Failed to relay Ethereum transaction');

      onInBlock();
    } catch (error) {
      onError(error as Error);
    } finally {
      await archiveApi?.disconnect();
    }
  };

  return useMutation({ mutationFn: relay });
}

export { useRelayEthTx };
