import { HexString } from '@gear-js/api';
import { useMemo } from 'react';
import { useReadContracts } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount, useInvalidateOnBlock } from '@/hooks';
import { FTAddressPair } from '@/types';
import { isUndefined } from '@/utils';

function useEthFTBalances(addresses: FTAddressPair[] | undefined) {
  const ethAccount = useEthAccount();

  const contracts = useMemo(
    () =>
      addresses?.map(([, address]) => ({
        address,
        abi: FUNGIBLE_TOKEN_ABI,
        functionName: 'balanceOf',
        args: [ethAccount.address],
      })),
    [addresses, ethAccount.address],
  );

  const getBalancesMap = (data: { result?: string | number | bigint }[]) => {
    if (!addresses) return;

    const entries = data.map(({ result }, pairIndex) => {
      const address = addresses[pairIndex][1];
      const balance = isUndefined(result) ? 0n : BigInt(result);

      return [address, balance] as const;
    });

    return Object.fromEntries(entries) as Record<HexString, bigint>;
  };

  const query = useReadContracts({
    contracts,
    query: {
      enabled: ethAccount.isConnected,
      select: getBalancesMap,
    },
  });

  useInvalidateOnBlock(query);

  return query;
}

export { useEthFTBalances };
