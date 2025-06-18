import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI, CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';

type Parameters = { amount: bigint; accountAddress: HexString };
type PermitParameters = Parameters & { permit: { deadline: bigint; v: number; r: HexString; s: HexString } };

function useTransfer(fee: bigint | undefined) {
  const { token } = useBridgeContext();

  const { writeContractAsync } = useWriteContract();
  const config = useConfig();

  const getGasLimit = ({ amount, accountAddress }: Parameters) => {
    definedAssert(fee, 'Fee');
    definedAssert(token?.address, 'Fungible token address');

    const encodedData = encodeFunctionData({
      abi: ERC20_MANAGER_ABI,
      functionName: 'requestBridgingPayingFee',
      args: [token.address, amount, accountAddress, CONTRACT_ADDRESS.ETH_BRIDGING_PAYMENT],
    });

    return estimateGas(config, {
      to: CONTRACT_ADDRESS.ERC20_MANAGER,
      data: encodedData,
      value: fee,
    });
  };

  const transfer = async ({ amount, accountAddress, ...params }: Parameters | PermitParameters) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const tx = { abi: ERC20_MANAGER_ABI, address: CONTRACT_ADDRESS.ERC20_MANAGER, value: fee };
    const permit = 'permit' in params ? params.permit : undefined;
    const permitArgs = permit ? ([permit.deadline, permit.v, permit.r, permit.s] as const) : undefined;

    const hash = permitArgs
      ? await writeContractAsync({
          ...tx,
          functionName: 'requestBridgingPayingFeeWithPermit',
          args: [token.address, amount, accountAddress, ...permitArgs, CONTRACT_ADDRESS.ETH_BRIDGING_PAYMENT],
        })
      : await writeContractAsync({
          ...tx,
          functionName: 'requestBridgingPayingFee',
          args: [token.address, amount, accountAddress, CONTRACT_ADDRESS.ETH_BRIDGING_PAYMENT],
        });

    return waitForTransactionReceipt(config, { hash });
  };

  return { ...useMutation({ mutationFn: transfer }), getGasLimit };
}

export { useTransfer };
