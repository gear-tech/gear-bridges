import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateFeesPerGas, estimateGas, watchContractEvent } from 'wagmi/actions';

import { isUndefined } from '@/utils';

import { BRIDGING_PAYMENT_ABI, ERROR_MESSAGE, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit(
  ftAddress: HexString | undefined,
  fee: bigint | undefined,
  allowance: bigint | undefined,
  _ftBalance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTransactionModal: (amount: string, receiver: string) => void,
) {
  const { writeContractAsync } = useWriteContract();
  const approve = useApprove(ftAddress);
  const config = useConfig();

  const getTransferGasLimit = (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not defined');

    const encodedData = encodeFunctionData({
      abi: BRIDGING_PAYMENT_ABI,
      functionName: 'requestBridging',
      args: [ftAddress, amount, accountAddress],
    });

    return estimateGas(config, {
      to: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
      data: encodedData,
      value: fee,
    });
  };

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not defined');
    if (isUndefined(fee)) throw new Error('Fee is not defined');
    if (isUndefined(allowance)) throw new Error('Allowance is not defined');
    if (isUndefined(accountBalance)) throw new Error('Account balance is not defined');

    const isApproveRequired = amount > allowance;

    const approveGasLimit = isApproveRequired ? await approve.getGasLimit(amount) : BigInt(0);
    const transferGasLimit = await getTransferGasLimit(amount, accountAddress);
    const gasLimit = approveGasLimit + transferGasLimit;

    const { maxFeePerGas } = await estimateFeesPerGas(config);
    const weiGasLimit = gasLimit * maxFeePerGas;

    const balanceToWithdraw = weiGasLimit + fee;

    if (balanceToWithdraw > accountBalance) throw new Error(ERROR_MESSAGE.NO_ACCOUNT_BALANCE);

    return { isApproveRequired, approveGasLimit, transferGasLimit };
  };

  const transfer = async (amount: bigint, accountAddress: HexString, gasLimit: bigint) => {
    if (!ftAddress) throw new Error('Fungible token address is not defined');
    if (!fee) throw new Error('Fee is not defined');

    return writeContractAsync({
      abi: BRIDGING_PAYMENT_ABI,
      address: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
      functionName: 'requestBridging',
      args: [ftAddress, amount, accountAddress],
      value: fee,
      gas: gasLimit,
    });
  };

  const watch = () =>
    new Promise<void>((resolve, reject) => {
      const onError = (error: Error) => {
        unwatch();
        reject(error);
      };

      const onLogs = () => {
        unwatch();
        resolve();
      };

      const address = ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS;
      const abi = BRIDGING_PAYMENT_ABI;

      const unwatch = watchContractEvent(config, { address, abi, eventName: 'FeePaid', onLogs, onError });
    });

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    const { isApproveRequired, approveGasLimit, transferGasLimit } = await validateBalance(amount, accountAddress);

    openTransactionModal(amount.toString(), accountAddress);

    if (isApproveRequired) {
      await approve.mutateAsync({ amount, gas: approveGasLimit });
    } else {
      approve.reset();
    }

    return transfer(amount, accountAddress, transferGasLimit).then(() => watch());
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, approve] as const;
}

export { useHandleEthSubmit };
