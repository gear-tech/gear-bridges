import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { WRAPPED_ETH_ABI } from '@/consts';
import { definedAssert } from '@/utils';

import { useBridgeContext } from '../../context';

function useMint() {
  const { token } = useBridgeContext();
  const { writeContractAsync } = useWriteContract();
  const config = useConfig();

  const mint = async ({ value }: { value: bigint }) => {
    definedAssert(token?.address, 'Fungible token address');

    const hash = await writeContractAsync({
      abi: WRAPPED_ETH_ABI,
      address: token.address, // only for wrapped eth
      functionName: 'deposit',
      value,
    });

    return waitForTransactionReceipt(config, { hash });
  };

  const getGasLimit = ({ value }: { value: bigint }) => {
    definedAssert(token?.address, 'Fungible token address');

    const data = encodeFunctionData({
      abi: WRAPPED_ETH_ABI,
      functionName: 'deposit',
    });

    return estimateGas(config, { to: token.address, data, value });
  };

  return { ...useMutation({ mutationFn: mint }), getGasLimit };
}

export { useMint };
