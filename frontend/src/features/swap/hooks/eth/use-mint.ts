import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { ETH_TOKEN_ABI } from '@/consts';
import { definedAssert } from '@/utils';

import { useBridgeContext } from '../../context';

function useMint() {
  const { token } = useBridgeContext();
  const { writeContractAsync } = useWriteContract();
  const config = useConfig();

  const mint = async ({ value, gas }: { value: bigint; gas: bigint }) => {
    definedAssert(token.address, 'Fungible token address');

    const hash = await writeContractAsync({
      abi: ETH_TOKEN_ABI,
      address: token.address, // only for wrapped eth
      functionName: 'tokenize',
      value,
      gas,
    });

    return waitForTransactionReceipt(config, { hash });
  };

  const getGasLimit = (value: bigint) => {
    definedAssert(token.address, 'Fungible token address');

    const data = encodeFunctionData({
      abi: ETH_TOKEN_ABI,
      functionName: 'tokenize',
    });

    return estimateGas(config, { to: token.address, data, value });
  };

  return { ...useMutation({ mutationFn: mint }), getGasLimit };
}

export { useMint };
