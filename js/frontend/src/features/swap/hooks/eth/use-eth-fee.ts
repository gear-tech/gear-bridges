import { formatEther } from 'viem';
import { useReadContract } from 'wagmi';

import { useNetworkType } from '@/context/network-type';
import { isUndefined } from '@/utils';

import { BRIDGING_PAYMENT_ABI } from '../../consts';

function useEthFee() {
  const { NETWORK_PRESET } = useNetworkType();

  const { data, isLoading } = useReadContract({
    abi: BRIDGING_PAYMENT_ABI,
    address: NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
    functionName: 'fee',
  });

  const bridgingFee = {
    value: data,
    formattedValue: !isUndefined(data) ? formatEther(data) : undefined,
  };

  return { bridgingFee, isLoading };
}

export { useEthFee };
