import { HexString } from '@gear-js/api';
import { formatUnits } from 'viem';
import { useReadContract } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount } from '@/hooks';
import { isUndefined } from '@/utils';

import { BALANCE_REFETCH_INTERVAL } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';

const abi = FUNGIBLE_TOKEN_ABI;

function useEthFTBalance(address: HexString | undefined, decimals: number | undefined) {
  const ethAccount = useEthAccount();

  // TODO: logger
  const { data, isLoading } = useReadContract({
    address,
    abi,
    functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_BALANCE,
    args: ethAccount.address ? [ethAccount.address] : undefined,

    query: {
      refetchInterval: BALANCE_REFETCH_INTERVAL,
      enabled: Boolean(address) && Boolean(ethAccount.address),
    },
  });

  const value = data;
  const formattedValue = !isUndefined(value) && !isUndefined(decimals) ? formatUnits(value, decimals) : undefined;

  return { value, formattedValue, decimals, isLoading };
}

export { useEthFTBalance };
