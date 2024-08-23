import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { Sails } from 'sails-js';
import { formatUnits } from 'viem';

import { isUndefined } from '@/utils';

import fungibleTokenIdlUrl from '../../assets/ft.idl?url';
import { BALANCE_REFETCH_INTERVAL } from '../../consts';

function useFungibleTokenBalance(address: HexString | undefined) {
  const { api, isApiReady } = useApi();

  const { account } = useAccount();
  const { decodedAddress } = account || {};

  const getBalance = async () => {
    if (!isApiReady) throw new Error('API is not initialized');
    if (!decodedAddress) throw new Error('Account is not found');
    if (!address) throw new Error('Fungible token address is not found');

    const response = await fetch(fungibleTokenIdlUrl);
    const idl = await response.text();

    const sails = (await Sails.new()).setApi(api).setProgramId(address);
    const parsedIdl = sails.parseIdl(idl);

    const { BalanceOf, Decimals } = parsedIdl.services.Erc20.queries;

    const balance = await BalanceOf<HexString>(decodedAddress, undefined, undefined, decodedAddress);
    const decimals = await Decimals<number>(decodedAddress);

    return { balance: BigInt(balance), decimals };
  };

  // TODO: logger
  const { data, isPending } = useQuery({
    queryKey: ['varaFungibleTokenBalance', address, decodedAddress],
    queryFn: getBalance,
    enabled: isApiReady && Boolean(account) && Boolean(address),
    refetchInterval: BALANCE_REFETCH_INTERVAL,
  });

  const { balance, decimals } = data || {};

  const value = balance;
  const formattedValue = !isUndefined(balance) && !isUndefined(decimals) ? formatUnits(balance, decimals) : undefined;

  const isLoading = isPending;

  return { value, formattedValue, decimals, isLoading };
}

export { useFungibleTokenBalance };
