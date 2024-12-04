import { HexString } from '@gear-js/api';
import { useMemo } from 'react';
import { useReadContracts } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount, useTokens } from '@/hooks';
import { isUndefined } from '@/utils';

function useEthFTBalances() {
  const ethAccount = useEthAccount();
  const { addresses, symbols, decimals } = useTokens();

  const contracts = useMemo(
    () =>
      addresses?.map(([, address]) => ({
        address: address.toString() as HexString,
        abi: FUNGIBLE_TOKEN_ABI,
        functionName: 'balanceOf',
        args: [ethAccount.address],
      })),
    [addresses, ethAccount.address],
  );

  return useReadContracts({
    contracts,
    query: {
      enabled: ethAccount.isConnected,
      select: (data) =>
        addresses &&
        symbols &&
        decimals &&
        data.map(({ result }, index) => {
          const address = addresses?.[index]?.[1].toString() as HexString;

          return {
            address,
            balance: isUndefined(result) ? 0n : BigInt(result),
            symbol: symbols[address],
            decimals: decimals[address],
          };
        }),
    },
  });
}

export { useEthFTBalances };
