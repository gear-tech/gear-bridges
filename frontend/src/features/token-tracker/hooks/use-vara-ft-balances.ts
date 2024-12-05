import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { ActorId, H160 } from 'sails-js';

import { VftProgram } from '@/consts';
import { TokenSupply } from '@/consts/sails/vft-manager';

function useVaraFTBalances(addresses: [ActorId, H160, TokenSupply][] | undefined) {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();

  const getBalances = async () => {
    if (!api) throw new Error('API not initialized');
    if (!account) throw new Error('Account not found');
    if (!addresses) throw new Error('Fungible tokens are not found');

    const result: Record<HexString, bigint> = {};

    for (const pair of addresses) {
      const address = pair[0].toString() as HexString;
      const balance = await new VftProgram(api, address).vft.balanceOf(account.decodedAddress);

      result[address] = balance;
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
