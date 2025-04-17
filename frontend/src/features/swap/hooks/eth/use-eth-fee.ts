import { formatEther } from 'viem';
import { useReadContract } from 'wagmi';

import { isUndefined } from '@/utils';

import { ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS, BRIDGING_PAYMENT_ABI } from '../../consts';

function useEthFee() {
  const { data, isLoading } = useReadContract({
    abi: BRIDGING_PAYMENT_ABI,
    address: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
    functionName: 'fee',
  });

  const fee = {
    value: data,
    formattedValue: !isUndefined(data) ? formatEther(data) : undefined,
  };

  return { fee, isLoading };
}

export { useEthFee };
