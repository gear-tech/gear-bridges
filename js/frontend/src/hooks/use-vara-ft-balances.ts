import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { VftProgram } from '@/consts';
import { useTokens } from '@/context';

function useVaraFTBalances() {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();
  const { tokens } = useTokens();

  const getBalances = async () => {
    if (!api) throw new Error('API not initialized');
    if (!account) throw new Error('Account not found');
    if (!tokens.vara) throw new Error('Fungible tokens are not found');

    const result: Record<HexString, bigint> = {};

    for (const token of tokens.vara) {
      const { address } = token;
      const balance = await new VftProgram(api, address).vft.balanceOf(account.decodedAddress);

      result[address] = balance;
    }

    return result;
  };

  return useQuery({
    queryKey: ['vara-ft-balances', account?.decodedAddress, tokens.vara],
    queryFn: getBalances,
    enabled: isApiReady && Boolean(account && tokens.vara),
    refetchInterval: 10000,
  });
}

export { useVaraFTBalances };
