import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { useNetworkType } from '@/context';
import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI } from '../../consts';
import { useBridgeContext } from '../../context';

type Parameters = {
  amount: bigint;
  accountAddress: HexString;
  permit?: { deadline: bigint; v: number; r: HexString; s: HexString };
};

function useTransfer(fee: bigint | undefined, shouldPayBridgingFee: boolean) {
  const { NETWORK_PRESET } = useNetworkType();
  const { token } = useBridgeContext();

  const { writeContractAsync } = useWriteContract();
  const config = useConfig();

  const getGasLimitWithFee = ({ amount, accountAddress }: Parameters) => {
    definedAssert(fee, 'Fee');
    definedAssert(token?.address, 'Fungible token address');

    const encodedData = encodeFunctionData({
      abi: ERC20_MANAGER_ABI,
      functionName: 'requestBridgingPayingFee',
      args: [token.address, amount, accountAddress, NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS],
    });

    return estimateGas(config, {
      to: NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS,
      data: encodedData,
      value: fee,
    });
  };

  const transferWithFee = async ({ amount, accountAddress, permit }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const tx = { abi: ERC20_MANAGER_ABI, address: NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS, value: fee };
    const permitArgs = permit ? ([permit.deadline, permit.v, permit.r, permit.s] as const) : undefined;

    const hash = permitArgs
      ? await writeContractAsync({
          ...tx,
          functionName: 'requestBridgingPayingFeeWithPermit',
          args: [
            token.address,
            amount,
            accountAddress,
            ...permitArgs,
            NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
          ],
        })
      : await writeContractAsync({
          ...tx,
          functionName: 'requestBridgingPayingFee',
          args: [token.address, amount, accountAddress, NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS],
        });

    return waitForTransactionReceipt(config, { hash });
  };

  const getGasLimitWithoutFee = ({ amount, accountAddress }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');

    const encodedData = encodeFunctionData({
      abi: ERC20_MANAGER_ABI,
      functionName: 'requestBridging',
      args: [token.address, amount, accountAddress],
    });

    return estimateGas(config, {
      to: NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS,
      data: encodedData,
    });
  };

  const transferWithoutFee = async ({ amount, accountAddress, permit }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');

    const tx = { abi: ERC20_MANAGER_ABI, address: NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS };
    const permitArgs = permit ? ([permit.deadline, permit.v, permit.r, permit.s] as const) : undefined;

    const hash = permitArgs
      ? await writeContractAsync({
          ...tx,
          functionName: 'requestBridgingWithPermit',
          args: [token.address, amount, accountAddress, ...permitArgs],
        })
      : await writeContractAsync({
          ...tx,
          functionName: 'requestBridging',
          args: [token.address, amount, accountAddress],
        });

    return waitForTransactionReceipt(config, { hash });
  };

  const transferWithoutFeeMutation = {
    ...useMutation({ mutationFn: transferWithoutFee }),
    getGasLimit: getGasLimitWithoutFee,
  };

  const transferWithFeeMutation = {
    ...useMutation({ mutationFn: transferWithFee }),
    getGasLimit: getGasLimitWithFee,
  };

  return shouldPayBridgingFee ? transferWithFeeMutation : transferWithoutFeeMutation;
}

export { useTransfer };
