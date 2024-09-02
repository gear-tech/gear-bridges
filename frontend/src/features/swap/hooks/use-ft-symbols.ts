import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { ActorId, H160 } from 'sails-js';
import { useConfig } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { VftProgram, FUNGIBLE_TOKEN_ABI } from '../consts';

function useFTSymbols(addresses: [ActorId, H160][] | undefined) {
  const { api, isApiReady } = useApi();
  const wagmiConfig = useConfig();

  const readVaraSymbol = (address: HexString) => {
    if (!api) throw new Error('Api is not initialized');

    return new VftProgram(api, address).vft.symbol();
  };

  const readEthSymbol = (address: HexString) =>
    readContract(wagmiConfig, { address, abi: FUNGIBLE_TOKEN_ABI, functionName: 'symbol' });

  const readSymbols = () => {
    if (!addresses) throw new Error('Fungible token addresses are not found');

    return Promise.all(
      addresses.map((pair) =>
        Promise.all([readVaraSymbol(pair[0].toString() as HexString), readEthSymbol(pair[1].toString() as HexString)]),
      ),
    );
  };

  return useQuery({
    queryKey: ['ftSymbols', addresses],
    queryFn: readSymbols,
    enabled: isApiReady && Boolean(addresses),
  });
}

export { useFTSymbols };
