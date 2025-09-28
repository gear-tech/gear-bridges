import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { ERC20_ABI } from '@/consts';
import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';
import { useBridgeContext } from '../../context';

const abi = ERC20_ABI;

function useApprove() {
  const { token } = useBridgeContext();
  const { address } = token || {};

  const config = useConfig();
  const { writeContractAsync } = useWriteContract();

  const getGasLimit = ({ amount }: { amount: bigint }) => {
    definedAssert(address, 'Fungible token address');

    const functionName = FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE;
    const args = [CONTRACT_ADDRESS.ERC20_MANAGER, amount] as const;
    const to = address;
    const data = encodeFunctionData({ abi, functionName, args });

    return estimateGas(config, { to, data });
  };

  const approve = async ({ amount }: { amount: bigint }) => {
    definedAssert(address, 'Fungible token address');

    const functionName = FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE;
    const args = [CONTRACT_ADDRESS.ERC20_MANAGER, amount] as const;

    const hash = await writeContractAsync({ address, abi, functionName, args });

    return waitForTransactionReceipt(config, { hash });
  };

  return { ...useMutation({ mutationFn: approve }), getGasLimit };
}

export { useApprove };
