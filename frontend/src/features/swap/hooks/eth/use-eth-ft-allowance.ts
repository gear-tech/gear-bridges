import { HexString } from '@gear-js/api';
import { useReadContract } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount, useInvalidateOnBlock } from '@/hooks';

import { useERC20ManagerAddress } from './use-erc20-manager-address';

function useEthFTAllowance(address: HexString | undefined) {
  const { data: erc20ManagerAddress } = useERC20ManagerAddress();
  const ethAccount = useEthAccount();

  const state = useReadContract({
    address,
    abi: FUNGIBLE_TOKEN_ABI,
    functionName: 'allowance',
    args: ethAccount.address && erc20ManagerAddress ? [ethAccount.address, erc20ManagerAddress] : undefined,
  });

  const { queryKey } = state;

  useInvalidateOnBlock({ queryKey });

  return state;
}

export { useEthFTAllowance };