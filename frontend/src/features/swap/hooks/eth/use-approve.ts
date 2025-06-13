import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { definedAssert } from '@/utils';

import { ERC20_MANAGER_CONTRACT_ADDRESS } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';
import { useBridgeContext } from '../../context';

const abi = FUNGIBLE_TOKEN_ABI;

function useApprove() {
  const { token } = useBridgeContext();
  const { address } = token || {};

  const config = useConfig();
  const { writeContractAsync } = useWriteContract();

  const getGasLimit = (amount: bigint) => {
    definedAssert(address, 'Fungible token address');

    const functionName = FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE;
    const args = [ERC20_MANAGER_CONTRACT_ADDRESS, amount] as const;
    const to = address;
    const data = encodeFunctionData({ abi, functionName, args });

    return estimateGas(config, { to, data });
  };

  const approve = async ({ amount, gas }: { amount: bigint; gas: bigint }) => {
    definedAssert(address, 'Fungible token address');

    const functionName = FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE;
    const args = [ERC20_MANAGER_CONTRACT_ADDRESS, amount] as const;

    const hash = await writeContractAsync({ address, abi, functionName, args, gas });

    return waitForTransactionReceipt(config, { hash });
  };

  return { ...useMutation({ mutationFn: approve }), getGasLimit };
}

export { useApprove };
