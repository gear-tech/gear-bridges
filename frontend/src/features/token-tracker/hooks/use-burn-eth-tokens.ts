import { useMutation } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { waitForTransactionReceipt, writeContract } from 'wagmi/actions';

import { ETH_TOKEN_ABI } from '@/consts';
import { ETH_WRAPPED_ETH_CONTRACT_ADDRESS } from '@/consts/env';

function useBurnEthTokens() {
  const config = useConfig();

  const burn = async (value: bigint) => {
    const hash = await writeContract(config, {
      address: ETH_WRAPPED_ETH_CONTRACT_ADDRESS,
      abi: ETH_TOKEN_ABI,
      functionName: 'release',
      args: [value],
    });

    return waitForTransactionReceipt(config, { hash });
  };

  return useMutation({ mutationFn: burn });
}

export { useBurnEthTokens };
