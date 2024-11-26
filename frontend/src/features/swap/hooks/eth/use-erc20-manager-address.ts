import { useReadContract } from 'wagmi';

import { BRIDGING_PAYMENT_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';

function useERC20ManagerAddress() {
  return useReadContract({
    abi: BRIDGING_PAYMENT_ABI,
    address: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
    functionName: 'getUnderlyingAddress',
  });
}

export { useERC20ManagerAddress };
