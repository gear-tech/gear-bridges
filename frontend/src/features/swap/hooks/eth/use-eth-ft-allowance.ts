import { HexString } from '@gear-js/api';
import { useReadContract } from 'wagmi';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount } from '@/hooks';

import { ALLOWANCE_REFETCH_INTERVAL } from '../../consts';

import { useERC20ManagerAddress } from './use-erc20-manager-address';

function useEthFTAllowance(address: HexString | undefined) {
  const { data: erc20ManagerAddress } = useERC20ManagerAddress();
  const ethAccount = useEthAccount();

  return useReadContract({
    address,
    abi: FUNGIBLE_TOKEN_ABI,
    functionName: 'allowance',
    args: ethAccount.address && erc20ManagerAddress ? [ethAccount.address, erc20ManagerAddress] : undefined,
    query: { refetchInterval: ALLOWANCE_REFETCH_INTERVAL },
  });
}

export { useEthFTAllowance };
