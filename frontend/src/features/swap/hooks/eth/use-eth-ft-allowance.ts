import { HexString } from '@gear-js/api';
import { useReadContract } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount, useInvalidateOnBlock } from '@/hooks';

import { ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';

function useEthFTAllowance(address: HexString | undefined) {
  const ethAccount = useEthAccount();

  const state = useReadContract({
    address,
    abi: FUNGIBLE_TOKEN_ABI,
    functionName: 'allowance',
    args: ethAccount.address ? [ethAccount.address, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS] : undefined,
    query: { enabled: Boolean(ethAccount.address) },
  });

  const { queryKey } = state;

  useInvalidateOnBlock({ queryKey });

  return state;
}

export { useEthFTAllowance };
