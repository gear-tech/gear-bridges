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

  const contracts = useMemo(
    () =>
      tokens.eth?.map(({ address }) => ({
        address,
        abi: FUNGIBLE_TOKEN_ABI,
        functionName: 'balanceOf',
        args: [ethAccount.address],
      })),
    [ethAccount.address, tokens.eth],
  );

  const getAddressToBalance = (data: { result?: string | number | bigint }[]) => {
    if (!tokens.eth) return;

    const entries = data.map(({ result }, index) => {
      const { address } = tokens.eth![index];
      const balance = isUndefined(result) ? 0n : BigInt(result);

      return [address, balance] as const;
    });

    return Object.fromEntries(entries) as Record<HexString, bigint>;
  };

  const query = useReadContracts({
    contracts,
    query: {
      enabled: ethAccount.isConnected,
      select: getAddressToBalance,
    },
  });

  useInvalidateOnBlock(query);

  return query;
}

export { useEthFTBalances };
