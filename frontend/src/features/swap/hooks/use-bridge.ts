import { HexString } from '@gear-js/api';
import { useProgram, useProgramQuery, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { useState } from 'react';
import { useConfig } from 'wagmi';
import { readContract } from 'wagmi/actions';

import {
  BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  BridgingPaymentProgram,
  FUNGIBLE_TOKEN_ABI,
  NETWORK_INDEX,
  VftGatewayProgram,
  VftProgram,
} from '../consts';

function useTokens() {
  const wagmiConfig = useConfig();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const { data: vftGatewayAddress } = useProgramQuery({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'vftGatewayAddress',
    args: [],
  });

  const { data: vftGatewayProgram } = useProgram({
    library: VftGatewayProgram,
    id: vftGatewayAddress?.toString() as HexString,
  });

  const { data: ftAdresses } = useProgramQuery({
    program: vftGatewayProgram,
    serviceName: 'vftGateway',
    functionName: 'varaToEthAddresses',
    args: [],
  });

  const { api, isApiReady } = useApi();

  const { data, isPending } = useQuery({
    queryKey: ['options', ftAdresses],

    queryFn: () => {
      if (!api || !ftAdresses) throw new Error('Api or ftAdresses is not ready');

      const promises = ftAdresses.map(async (addressPair) => {
        const varaAddress = addressPair[0].toString() as HexString;
        const ethAddress = addressPair[1].toString() as HexString;

        // TODO: read decimals only for selected?
        const vftProgram = new VftProgram(api, varaAddress);
        const varaSymbol = await vftProgram.vft.symbol();
        const varaDecimals = await vftProgram.vft.decimals();

        const ethSymbol = await readContract(wagmiConfig, {
          address: ethAddress.toString() as HexString,
          abi: FUNGIBLE_TOKEN_ABI,
          functionName: 'symbol',
        });

        const ethDecimals = await readContract(wagmiConfig, {
          address: ethAddress.toString() as HexString,
          abi: FUNGIBLE_TOKEN_ABI,
          functionName: 'decimals',
        });

        return [
          { address: varaAddress, symbol: varaSymbol, decimals: varaDecimals },
          { address: ethAddress, symbol: ethSymbol, decimals: ethDecimals },
        ];
      });

      return Promise.all(promises);
    },

    enabled: isApiReady && Boolean(ftAdresses),
  });

  const isLoading = isPending;

  return { data, isLoading };
}

const getOptions = (data: ReturnType<typeof useTokens>['data']) => {
  const varaOptions: { label: string; value: string }[] = [];
  const ethOptions: { label: string; value: string }[] = [];

  data?.forEach(([vara, eth], index) => {
    varaOptions.push({ value: index.toString(), label: vara.symbol });
    ethOptions.push({ value: index.toString(), label: eth.symbol });
  });

  return { varaOptions, ethOptions };
};

function useBridge(networkIndex: number) {
  const isVaraNetwork = networkIndex === NETWORK_INDEX.VARA;

  const tokens = useTokens();
  const [pair, setPair] = useState('0');

  const { varaOptions, ethOptions } = getOptions(tokens.data);
  const options = { from: isVaraNetwork ? varaOptions : ethOptions, to: isVaraNetwork ? ethOptions : varaOptions };

  const bridge = tokens.data?.[Number(pair)][networkIndex];
  const { address } = bridge || {};

  const nativeSymbol = isVaraNetwork ? 'VARA' : 'ETH';
  const symbol = { value: bridge?.symbol, native: nativeSymbol };

  return { address, options, symbol, pair: { value: pair, set: setPair }, isLoading: tokens.isLoading };
}

export { useBridge };
