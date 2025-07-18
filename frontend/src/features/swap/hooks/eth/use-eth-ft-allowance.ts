import { HexString } from '@gear-js/api';
import { useReadContract } from 'wagmi';

import { ERC20_ABI } from '@/consts';
import { useEthAccount, useInvalidateOnBlock } from '@/hooks';

import { CONTRACT_ADDRESS } from '../../consts';

function useEthFTAllowance(address: HexString | undefined) {
  const ethAccount = useEthAccount();

  const state = useReadContract({
    address,
    abi: ERC20_ABI,
    functionName: 'allowance',
    args: ethAccount.address ? [ethAccount.address, CONTRACT_ADDRESS.ERC20_MANAGER] : undefined,
    query: { enabled: Boolean(ethAccount.address) },
  });

  const { queryKey } = state;

  useInvalidateOnBlock({ queryKey });

  return state;
}

export { useEthFTAllowance };
