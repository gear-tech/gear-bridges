import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData, formatEther } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateFeesPerGas, estimateGas, watchContractEvent } from 'wagmi/actions';

import { isUndefined } from '@/utils';

import { BRIDGING_PAYMENT_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';

function useHandleEthSubmit(ftAddress: HexString | undefined, fee: bigint | undefined, allowance: bigint | undefined) {
  const { writeContractAsync } = useWriteContract();
  const approve = useApprove(ftAddress);
  const config = useConfig();

  const requestBridging = async (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not defined');
    if (!fee) throw new Error('Fee is not defined');

    const { maxFeePerGas } = await estimateFeesPerGas(config);

    const encodedData = encodeFunctionData({
      abi: BRIDGING_PAYMENT_ABI,
      functionName: 'requestBridging',
      args: [ftAddress, amount, accountAddress],
    });

    const gasLimit = await estimateGas(config, {
      to: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
      data: encodedData,
      value: fee,
    });

    const weiGasLimit = gasLimit * maxFeePerGas;
    const ethGasLimit = formatEther(weiGasLimit);

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
    if (isUndefined(allowance)) throw new Error('Allowance is not defined');

    if (amount > allowance) {
      await approve.mutateAsync(amount);
    } else {
      approve.reset();
    }

    return requestBridging(amount, accountAddress).then(() => watch());
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, approve] as const;
}

export { useHandleEthSubmit };
