import { HexString } from '@gear-js/api';
import { useReadContract } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '../consts';

import { useInvalidateOnBlock } from './common';
import { useEthAccount } from './use-eth-account';

function useEthFTBalance(address: HexString | undefined) {
  const ethAccount = useEthAccount();
  const enabled = Boolean(address) && Boolean(ethAccount.address);

  const state = useReadContract({
    address,
    abi: FUNGIBLE_TOKEN_ABI,
    functionName: 'balanceOf',
    args: ethAccount.address ? [ethAccount.address] : undefined,

    query: { enabled },
  });

  const { queryKey } = state;
  useInvalidateOnBlock({ queryKey, enabled });

  return state;
}

export { useEthFTBalance };
