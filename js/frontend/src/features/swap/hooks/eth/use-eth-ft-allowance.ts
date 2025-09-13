import { HexString } from '@gear-js/api';
import { useReadContract } from 'wagmi';

import { ERC20_ABI } from '@/consts';
import { useEthAccount, useInvalidateOnBlock } from '@/hooks';

import { CONTRACT_ADDRESS } from '../../consts';

const DUMMY_ETH_ADDRESS = '0x000000000000000000000000000000000000dEaD';

function useEthFTAllowance(address: HexString | undefined) {
  const ethAccount = useEthAccount();

  const state = useReadContract({
    address,
    abi: ERC20_ABI,
    functionName: 'allowance',
    args: [ethAccount.address || DUMMY_ETH_ADDRESS, CONTRACT_ADDRESS.ERC20_MANAGER],

    // it's probably worth to check isConnecting too, but there is a bug:
    // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
    query: { enabled: !ethAccount.isReconnecting },
  });

  const { queryKey } = state;

  useInvalidateOnBlock({ queryKey });

  return state;
}

export { useEthFTAllowance };
