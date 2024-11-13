import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { ActorId, H160 } from 'sails-js';
import { useConfig } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { VftProgram, FUNGIBLE_TOKEN_ABI } from '@/consts';

function useFTDecimals(addresses: [ActorId, H160][] | undefined) {
  const { api, isApiReady } = useApi();
  const wagmiConfig = useConfig();

  const readVaraDecimals = (address: HexString) => {
    if (!api) throw new Error('Api is not initialized');

    return new VftProgram(api, address).vft.decimals();
  };

  const readEthDecimals = (address: HexString) =>
    readContract(wagmiConfig, { address, abi: FUNGIBLE_TOKEN_ABI, functionName: 'decimals' });

  const readDecimals = async () => {
    if (!addresses) throw new Error('Fungible token addresses are not found');

    const result: Record<HexString, number> = {};

    for (const pair of addresses) {
      const varaAddress = pair[0].toString() as HexString;
      const ethAddress = pair[1].toString() as HexString;

      const [varaDecimals, ethDecimals] = await Promise.all([
        readVaraDecimals(varaAddress),
        readEthDecimals(ethAddress),
      ]);

      result[varaAddress] = varaDecimals;
      result[ethAddress] = ethDecimals;
    }

    return result;
  };

  return useQuery({
    queryKey: ['ftDecimals'],
    queryFn: readDecimals,
    enabled: isApiReady && Boolean(addresses),
  });
}

export { useFTDecimals };
