import { HexString } from '@gear-js/api';
import { useAccount, useProgram, useProgramQuery } from '@gear-js/react-hooks';
import { formatUnits } from 'viem';

import { isUndefined } from '@/utils';

import { BALANCE_REFETCH_INTERVAL, VftProgram } from '../../consts';

const SERVICE_NAME = 'vft';

const QUERY_NAME = {
  BALANCE: 'balanceOf',
  DECIMALS: 'decimals',
} as const;

function useFungibleTokenBalance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useProgram({
    library: VftProgram,
    id: address,
  });

  const { data: balance, isPending: isBalancePending } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME,
    functionName: QUERY_NAME.BALANCE,
    args: [account!.decodedAddress],
    query: { enabled: Boolean(account), refetchInterval: BALANCE_REFETCH_INTERVAL },
  });

  const { data: decimals, isPending: isDecimalsPending } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME,
    functionName: QUERY_NAME.DECIMALS,
    args: [],
  });

  const value = balance;
  const formattedValue = !isUndefined(balance) && !isUndefined(decimals) ? formatUnits(balance, decimals) : undefined;

  const isLoading = isBalancePending || isDecimalsPending;

  return { value, formattedValue, decimals, isLoading };
}

export { useFungibleTokenBalance };
