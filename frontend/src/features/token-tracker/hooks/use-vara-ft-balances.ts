import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { VftProgram } from '@/consts';
import { FTAddressPair } from '@/types';

function useVaraFTBalances(addresses: FTAddressPair[] | undefined) {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();

  const getBalances = async () => {
    if (!api) throw new Error('API not initialized');
    if (!account) throw new Error('Account not found');
    if (!addresses) throw new Error('Fungible tokens are not found');

    const result: Record<HexString, { balance: bigint; pairIndex: number }> = {};

    for (const [pairIndex, [address]] of addresses.entries()) {
      const balance = await new VftProgram(api, address).vft.balanceOf(account.decodedAddress);

      result[address] = { balance, pairIndex };
    }

    return result;
  };

  return useQuery({
    queryKey: ['vara-ft-balances', account?.decodedAddress, addresses],
    queryFn: getBalances,
    enabled: isApiReady && Boolean(account && addresses),
  });
}

export { useVaraFTBalances };
