import { HexString } from '@gear-js/api';
import { formatUnits } from 'viem';
import { useReadContracts } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount } from '@/hooks';
import { isUndefined } from '@/utils';

import { BALANCE_REFETCH_INTERVAL } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';

const abi = FUNGIBLE_TOKEN_ABI;

function useEthFTBalance(address: HexString | undefined) {
  const ethAccount = useEthAccount();

  // TODO: logger
  const { data, isPending } = useReadContracts({
    contracts: [
      {
        address,
        abi,
        functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_BALANCE,
        args: ethAccount.address ? [ethAccount.address] : undefined,
      },

      { address, abi, functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_DECIMALS },
    ],

    query: {
      refetchInterval: BALANCE_REFETCH_INTERVAL,
      enabled: Boolean(address) && Boolean(ethAccount.address),
    },
  });

  const value = data?.[0].result;
  const decimals = data?.[1].result;

  const formattedValue = !isUndefined(value) && !isUndefined(decimals) ? formatUnits(value, decimals) : undefined;

  const isLoading = ethAccount.isConnected && isPending;

  return { value, formattedValue, decimals, isLoading };
}

export { useEthFTBalance };
