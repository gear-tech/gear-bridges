import { HexString } from '@gear-js/api';
import { useAccount, useProgram, useProgramQuery } from '@gear-js/react-hooks';
import { formatUnits } from 'viem';

import { VftProgram } from '@/consts';
import { isUndefined } from '@/utils';

import { BALANCE_REFETCH_INTERVAL, QUERY_NAME, SERVICE_NAME } from '../../consts';

function useVaraFTBalance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useProgram({
    library: VftProgram,
    id: address,
  });

  const { data: balance, isPending: isBalancePending } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME.VFT,
    functionName: QUERY_NAME.BALANCE,
    args: [account?.decodedAddress || '0x00'],
    query: { enabled: Boolean(account), refetchInterval: BALANCE_REFETCH_INTERVAL },
  });

  const { data: decimals, isPending: isDecimalsPending } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME.VFT,
    functionName: QUERY_NAME.DECIMALS,
    args: [],
  });

  const value = balance;
  const formattedValue = !isUndefined(balance) && !isUndefined(decimals) ? formatUnits(balance, decimals) : undefined;

  const isLoading = isBalancePending || isDecimalsPending;

  return { value, formattedValue, decimals, isLoading };
}

export { useVaraFTBalance };
