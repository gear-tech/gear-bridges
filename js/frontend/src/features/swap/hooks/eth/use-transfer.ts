import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { useNetworkType } from '@/context/network-type';
import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI } from '../../consts';
import { useBridgeContext } from '../../context';
import { assertSuccessfulReceipt } from '../../submission';

type Parameters = {
  amount: bigint;
  accountAddress: HexString;
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
      args: [token.address, amount, accountAddress, NETWORK_PRESET.ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
    });

    return estimateGas(config, {
      to: NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS,
      data: encodedData,
      value: fee,
    });
  };

  const transferWithFee = async ({ amount, accountAddress }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const tx = {
      chainId: NETWORK_PRESET.ETH_CHAIN_ID,
      abi: ERC20_MANAGER_ABI,
      address: NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS,
      value: fee,
    };
    const hash = await writeContractAsync({
      ...tx,
      functionName: 'requestBridgingPayingFee',
      args: [token.address, amount, accountAddress, NETWORK_PRESET.ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
    });

    return assertSuccessfulReceipt(await waitForTransactionReceipt(config, { hash }), hash);
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

  const transferWithoutFee = async ({ amount, accountAddress }: Parameters) => {
    definedAssert(token?.address, 'Fungible token address');

    const tx = {
      chainId: NETWORK_PRESET.ETH_CHAIN_ID,
      abi: ERC20_MANAGER_ABI,
      address: NETWORK_PRESET.ERC20_MANAGER_CONTRACT_ADDRESS,
    };
    const hash = await writeContractAsync({
      ...tx,
      functionName: 'requestBridging',
      args: [token.address, amount, accountAddress],
    });

    return assertSuccessfulReceipt(await waitForTransactionReceipt(config, { hash }), hash);
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
