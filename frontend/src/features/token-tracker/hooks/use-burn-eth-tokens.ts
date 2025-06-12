import { useMutation } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { waitForTransactionReceipt, writeContract } from 'wagmi/actions';

import { ETH_TOKEN_ABI } from '@/consts';
import { useTokens } from '@/context';
import { definedAssert } from '@/utils';

function useBurnEthTokens() {
  const { tokens } = useTokens();

  // TODO: active filter
  const wrappedEthAddress = tokens?.find(
    ({ network, isActive, isNative }) => isActive && isNative && network === 'eth',
  )?.address;

  const config = useConfig();

  const burn = async (value: bigint) => {
    definedAssert(wrappedEthAddress, 'ETH token address');

    const hash = await writeContract(config, {
      address: wrappedEthAddress,
      abi: ETH_TOKEN_ABI,
      functionName: 'release',
      args: [value],
    });

    return waitForTransactionReceipt(config, { hash });
  };

  return useMutation({ mutationFn: burn });
}

export { useBurnEthTokens };
