import { useMutation } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { waitForTransactionReceipt, writeContract } from 'wagmi/actions';

import { WRAPPED_ETH_ABI } from '@/consts';
import { useTokens } from '@/context';
import { definedAssert } from '@/utils';

function useBurnEthTokens() {
  const { nativeToken } = useTokens();

  const config = useConfig();

  const burn = async (value: bigint) => {
    definedAssert(nativeToken.eth, 'ETH token address');

    const hash = await writeContract(config, {
      address: nativeToken.eth.address,
      abi: WRAPPED_ETH_ABI,
      functionName: 'withdraw',
      args: [value],
    });

    return waitForTransactionReceipt(config, { hash });
  };

  return useMutation({ mutationFn: burn });
}

export { useBurnEthTokens };
