import { HexString } from '@gear-js/api';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS, ERC20_MANAGER_CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';

function useTransfer(fee: bigint | undefined) {
  const { token } = useBridgeContext();

  const { writeContractAsync } = useWriteContract();
  const config = useConfig();

  const getGasLimit = (amount: bigint, accountAddress: HexString) => {
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

  const transfer = async (amount: bigint, accountAddress: HexString, gasLimit: bigint | undefined) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const hash = await writeContractAsync({
      abi: ERC20_MANAGER_ABI,
      address: ERC20_MANAGER_CONTRACT_ADDRESS,
      functionName: 'requestBridgingPayingFee',
      args: [token.address, amount, accountAddress, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
      value: fee,
      gas: gasLimit,
    });

    return waitForTransactionReceipt(config, { hash });
  };

  return { mutateAsync: transfer, getGasLimit };
}

export { useTransfer };
