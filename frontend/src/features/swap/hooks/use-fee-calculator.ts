import { useAlert } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';
import { useEffect } from 'react';
import { FEE_CALCULATOR_URL, FEE_DECIMALS, NETWORK_NAME } from '@/consts';
import { NetworkName, FeeCalculator } from '../types';
import { FeeCalculatorResponse } from '../types/hooks';
import { formatUnits } from 'viem';

type Params = {
  networkName: NetworkName;
};

function useFeeCalculator({ networkName }: Params) {
  const targetNetwork = networkName === NETWORK_NAME.VARA ? 'eth' : 'vara';
  const alert = useAlert();

  const getFee = async () => {
    const response = await fetch(`${FEE_CALCULATOR_URL}/${targetNetwork}`);
    if (!response.ok) throw new Error('Fee calculator is not available');

    const json = (await response.json()) as FeeCalculatorResponse;

    return json;
  };

  const prepareData = (originalData: FeeCalculatorResponse) => {
    const value = BigInt(originalData.fee);
    return {
      ...originalData,
      fee: {
        value: value,
        formattedValue: formatUnits(value, FEE_DECIMALS[networkName]),
      },
    } as FeeCalculator;
  };

  const { data, refetch, isFetching, error } = useQuery({
    queryKey: ['feeCalculator', targetNetwork],
    queryFn: getFee,
    select: prepareData,
  });

  useEffect(() => {
    if (error) {
      alert.error('Fee calculator is not available');
    }
  }, [error]);

  const isFeeLoading = isFetching;

  return { feeCalculatorData: data, isFeeLoading, refetch, error };
}

export { useFeeCalculator };
