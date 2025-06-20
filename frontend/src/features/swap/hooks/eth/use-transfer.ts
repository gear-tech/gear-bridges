import { HexString } from '@gear-js/api';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, waitForTransactionReceipt } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { ERC20_MANAGER_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS, ERC20_MANAGER_CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';

type Parameters = { amount: bigint; accountAddress: HexString };
type TxParameters = Parameters & { gasLimit: bigint | undefined };
type PermitTxParameters = Parameters & { permit: { deadline: bigint; v: number; r: HexString; s: HexString } };

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

  const transfer = async ({ amount, accountAddress, gasLimit }: TxParameters) => {
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

  const transferWithPermit = async ({ amount, accountAddress, permit }: PermitTxParameters) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');

    const { deadline, v, r, s } = permit;

    const hash = await writeContractAsync({
      abi: ERC20_MANAGER_ABI,
      address: ERC20_MANAGER_CONTRACT_ADDRESS,
      functionName: 'requestBridgingPayingFeeWithPermit',
      args: [token.address, amount, accountAddress, deadline, v, r, s, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS],
      value: fee,
    });

    return waitForTransactionReceipt(config, { hash });
  };

  return { mutateAsync: transfer, mutateWithPermitAsync: transferWithPermit, getGasLimit };
}

export { useTransfer };
