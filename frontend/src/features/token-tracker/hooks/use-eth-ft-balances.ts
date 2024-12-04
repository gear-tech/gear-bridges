import { HexString } from '@gear-js/api';
import { useMemo } from 'react';
import { ActorId, H160 } from 'sails-js';
import { useReadContracts } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { TokenSupply } from '@/consts/sails/vft-manager';
import { useEthAccount } from '@/hooks';
import { isUndefined } from '@/utils';

function useEthFTBalances(addresses: [ActorId, H160, TokenSupply][] | undefined) {
  const ethAccount = useEthAccount();

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

  const getBalancesMap = (data: { result?: string | number | bigint }[]) => {
    if (!addresses) return;

    const entries = data.map(({ result }, index) => {
      const address = addresses[index][1].toString() as HexString;
      const balance = isUndefined(result) ? 0n : BigInt(result);

      return [address, balance] as const;
    });

    return Object.fromEntries(entries);
  };

  return useReadContracts({
    contracts,
    query: {
      enabled: ethAccount.isConnected,
      select: getBalancesMap,
    },
  });
}

export { useEthFTBalances };
