import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI, CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';

type Parameters = {
  amount: bigint;
  accountAddress: HexString;
  accountOverride?: HexString;
  permit?: { deadline: bigint; v: number; r: HexString; s: HexString };
};

function useTransfer(fee: bigint | undefined) {
  const { token } = useBridgeContext();

  const { writeContractAsync } = useWriteContract();
  const config = useConfig();

  const getGasLimitWithFee = ({ amount, accountAddress, accountOverride }: Parameters) => {
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
      account: accountOverride,
    });
  };

  const transferWithFee = async ({ amount, accountAddress, permit }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const tx = { abi: ERC20_MANAGER_ABI, address: CONTRACT_ADDRESS.ERC20_MANAGER, value: fee };
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

  const getGasLimitWithoutFee = ({ amount, accountAddress, accountOverride }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');

    const encodedData = encodeFunctionData({
      abi: ERC20_MANAGER_ABI,
      functionName: 'requestBridging',
      args: [token.address, amount, accountAddress],
    });

    return estimateGas(config, {
      to: CONTRACT_ADDRESS.ERC20_MANAGER,
      data: encodedData,
      account: accountOverride,
    });
  };

  const transferWithoutFee = async ({ amount, accountAddress, permit }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');

    const tx = { abi: ERC20_MANAGER_ABI, address: CONTRACT_ADDRESS.ERC20_MANAGER };
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

  return {
    transferWithoutFee: { ...useMutation({ mutationFn: transferWithoutFee }), getGasLimit: getGasLimitWithoutFee },
    transferWithFee: { ...useMutation({ mutationFn: transferWithFee }), getGasLimit: getGasLimitWithFee },
  };
}

export { useTransfer };
