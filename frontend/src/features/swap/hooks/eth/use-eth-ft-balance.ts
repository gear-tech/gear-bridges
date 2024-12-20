import { HexString } from '@gear-js/api';
import { useReadContract } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount, useInvalidateOnBlock } from '@/hooks';

import { FUNCTION_NAME } from '../../consts/eth';

const abi = FUNGIBLE_TOKEN_ABI;

function useEthFTBalance(address: HexString | undefined) {
  const ethAccount = useEthAccount();
  const enabled = Boolean(address) && Boolean(ethAccount.address);

  const state = useReadContract({
    address,
    abi,
    functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_BALANCE,
    args: ethAccount.address ? [ethAccount.address] : undefined,

    query: { enabled },
  });

  const { queryKey } = state;
  useInvalidateOnBlock({ queryKey, enabled });

  return state;
}

export { useEthFTBalance };
