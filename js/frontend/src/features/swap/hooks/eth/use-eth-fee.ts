import { useReadContract } from 'wagmi';

import { useNetworkType } from '@/context/network-type';

import { BRIDGING_PAYMENT_ABI } from '../../consts';

function useEthFee() {
  const { NETWORK_PRESET } = useNetworkType();

  const { data: bridgingFee, isLoading } = useReadContract({
    abi: BRIDGING_PAYMENT_ABI,
    address: NETWORK_PRESET.ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
    functionName: 'fee',
  });

  return { bridgingFee, isLoading };
}

export { useEthFee };
