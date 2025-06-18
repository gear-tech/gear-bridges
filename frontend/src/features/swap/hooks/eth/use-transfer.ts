import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS, ERC20_MANAGER_CONTRACT_ADDRESS } from '../../consts';
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
      args: [token.address, amount, accountAddress, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
    });

    return estimateGas(config, {
      to: ERC20_MANAGER_CONTRACT_ADDRESS,
      data: encodedData,
      value: fee,
    });
  };

  const transfer = async ({ amount, accountAddress, ...params }: Parameters | PermitParameters) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const withPermit = 'permit' in params;

    const hash = withPermit
      ? await writeContractAsync({
          abi: ERC20_MANAGER_ABI,
          address: ERC20_MANAGER_CONTRACT_ADDRESS,
          functionName: 'requestBridgingPayingFeeWithPermit',
          args: [
            token.address,
            amount,
            accountAddress,
            params.permit.deadline,
            params.permit.v,
            params.permit.r,
            params.permit.s,
            ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
          ],
          value: fee,
        })
      : await writeContractAsync({
          abi: ERC20_MANAGER_ABI,
          address: ERC20_MANAGER_CONTRACT_ADDRESS,
          functionName: 'requestBridgingPayingFee',
          args: [token.address, amount, accountAddress, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
          value: fee,
        });

    return waitForTransactionReceipt(config, { hash });
  };

  return { ...useMutation({ mutationFn: transfer }), getGasLimit };
}

export { useTransfer };
