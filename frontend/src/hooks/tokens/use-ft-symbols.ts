import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { ActorId, H160 } from 'sails-js';
import { useConfig } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { VftProgram, FUNGIBLE_TOKEN_ABI } from '@/consts';

function useFTSymbols(addresses: [ActorId, H160, 'ethereum' | 'gear'][] | undefined) {
  const { api, isApiReady } = useApi();
  const wagmiConfig = useConfig();

  const readVaraSymbol = (address: HexString) => {
    if (!api) throw new Error('Api is not initialized');

    return new VftProgram(api, address).vft.symbol();
  };

  const readEthSymbol = (address: HexString) =>
    readContract(wagmiConfig, { address, abi: FUNGIBLE_TOKEN_ABI, functionName: 'symbol' });

  const readSymbols = async () => {
    if (!addresses) throw new Error('Fungible token addresses are not found');

    const result: Record<HexString, string> = {};

    for (const pair of addresses) {
      const varaAddress = pair[0].toString() as HexString;
      const ethAddress = pair[1].toString() as HexString;

      const [varaSymbol, ethSymbol] = await Promise.all([readVaraSymbol(varaAddress), readEthSymbol(ethAddress)]);

      result[varaAddress] = varaSymbol;
      result[ethAddress] = ethSymbol;
    }

    return result;
  };

  return useQuery({
    queryKey: ['ftSymbols'],
    queryFn: readSymbols,
    enabled: isApiReady && Boolean(addresses),
  });
}

export { useFTSymbols };
