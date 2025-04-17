import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { readContract } from 'wagmi/actions';

import { VftProgram, FUNGIBLE_TOKEN_ABI } from '@/consts';
import { FTAddressPair } from '@/types';

function useFTDecimals(addresses: FTAddressPair[] | undefined) {
  const { api, isApiReady } = useApi();
  const wagmiConfig = useConfig();

  const readVaraDecimals = (address: HexString) => {
    if (!api) throw new Error('Api is not initialized');

    return new VftProgram(api, address).vftMetadata.decimals();
  };

  const readEthDecimals = (address: HexString) =>
    readContract(wagmiConfig, { address, abi: FUNGIBLE_TOKEN_ABI, functionName: 'decimals' });

  const readDecimals = async () => {
    if (!addresses) throw new Error('Fungible token addresses are not found');

    const result: Record<HexString, number> = {};

    for (const [varaAddress, ethAddress] of addresses) {
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
