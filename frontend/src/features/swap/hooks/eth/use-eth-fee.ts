import { formatEther } from 'viem';
import { useReadContract } from 'wagmi';

import { isUndefined } from '@/utils';

import { CONTRACT_ADDRESS, BRIDGING_PAYMENT_ABI } from '../../consts';

function useEthFee() {
  const { data, isLoading } = useReadContract({
    abi: BRIDGING_PAYMENT_ABI,
    address: CONTRACT_ADDRESS.ETH_BRIDGING_PAYMENT,
    functionName: 'fee',
  });

  const bridgingFee = {
    value: data,
    formattedValue: !isUndefined(data) ? formatEther(data) : undefined,
  };

  return { bridgingFee, isLoading };
}

export { useEthFee };
