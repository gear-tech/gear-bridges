import { formatEther } from 'viem';
import { useReadContracts } from 'wagmi';

import { isUndefined } from '@/utils';

import { ABI, FUNCTION_NAME } from '../../consts';
import { Contract } from '../../types';

function useEthConfig({ address }: Contract) {
  // TODO: logger
  const abi = ABI;

  const { data, isLoading } = useReadContracts({
    contracts: [
      { abi, address, functionName: FUNCTION_NAME.FEE },
      { abi, address, functionName: FUNCTION_NAME.MIN_AMOUNT },
      { abi, address, functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_ADDRESS },
    ],
  });

  const fee = data?.[0]?.result;
  const minValue = data?.[1]?.result;
  const ftAddress = data?.[2]?.result;

  const formattedFee = !isUndefined(fee) ? formatEther(fee) : undefined;

  return { fee: { value: fee, formattedValue: formattedFee }, minValue, ftAddress, isLoading };
}

export { useEthConfig };
