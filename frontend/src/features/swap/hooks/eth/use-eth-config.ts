import { HexString } from '@gear-js/api';
import { formatEther } from 'viem';
import { useReadContract } from 'wagmi';

import { isUndefined } from '@/utils';

import { ABI, FUNCTION_NAME } from '../../consts';

function useEthConfig(address: HexString | undefined) {
  // TODO: logger
  const abi = ABI;

  const { data, isLoading } = useReadContract({
    abi,
    address,
    functionName: FUNCTION_NAME.MIN_AMOUNT,
  });

  const fee = {
    value: data,
    formattedValue: !isUndefined(data) ? formatEther(data) : undefined,
  };

  return { fee, isLoading };
}

export { useEthConfig };
