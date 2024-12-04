import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { VftProgram } from '@/consts';
import { useTokens } from '@/hooks';

function useVaraFTBalances() {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();
  const { addresses, decimals, symbols } = useTokens();

  const getBalances = async () => {
    if (!api) throw new Error('API not initialized');
    if (!account) throw new Error('Account not found');
    if (!addresses || !symbols || !decimals) throw new Error('Fungible tokens are not found');

    const balancePromises = addresses.map(async ([_address]) => {
      const address = _address.toString() as HexString;

      return {
        address,
        balance: await new VftProgram(api, address).vft.balanceOf(account.decodedAddress),
        symbol: symbols[address],
        decimals: decimals[address],
      };
    });

    return Promise.all(balancePromises);
  };

  return useQuery({
    queryKey: ['vara-ft-balances', account?.decodedAddress, addresses, symbols, decimals],
    queryFn: getBalances,
    enabled: isApiReady && Boolean(account && addresses && symbols && decimals),
  });
}

export { useVaraFTBalances };
