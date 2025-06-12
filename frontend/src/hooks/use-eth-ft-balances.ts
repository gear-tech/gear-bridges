import { HexString } from '@gear-js/api';
import { useMemo } from 'react';
import { useReadContracts } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useTokens } from '@/context';
import { useEthAccount, useInvalidateOnBlock } from '@/hooks';
import { isUndefined } from '@/utils';

function useEthFTBalances() {
  const ethAccount = useEthAccount();

  const { tokens } = useTokens();

  // TODO: active filter
  const addresses = tokens
    ?.filter(({ isActive, network }) => isActive && network === 'eth')
    .map(({ address }) => address);

  const contracts = useMemo(
    () =>
      addresses?.map((address) => ({
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
      const address = addresses[pairIndex];
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
