import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { encodeFunctionData, WatchContractEventOnLogsParameter } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { estimateGas, watchContractEvent } from 'wagmi/actions';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount } from '@/hooks';

import { ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS, EVENT_NAME } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';

const abi = FUNGIBLE_TOKEN_ABI;

function useApprove(address: HexString | undefined) {
  const ethAccount = useEthAccount();
  const config = useConfig();
  const { writeContractAsync } = useWriteContract();

  // maybe better to use waitForTransactionReceipt,
  // but feels like it's getting fired before approval in contract
  const watch = (amount: bigint) =>
    new Promise<bigint>((resolve, reject) => {
      const eventName = EVENT_NAME.APPROVAL;
      const args = { owner: ethAccount.address, spender: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS };

      const onLogs = (logs: WatchContractEventOnLogsParameter<typeof abi, typeof EVENT_NAME.APPROVAL>) =>
        logs.forEach(({ args: { value = 0n } }) => {
          unwatch();

          if (value < amount) return reject(new Error('Approved value is less than the required amount'));

          resolve(value);
        });

      const onError = (error: Error) => {
        unwatch();
        reject(error);
      };

      const unwatch = watchContractEvent(config, { address, abi, eventName, args, onLogs, onError });
    });

  const getGasLimit = (amount: bigint) => {
    if (!address) throw new Error('Fungible token address is not defined');

    const functionName = FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE;
    const args = [ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS, amount] as const;
    const to = address;
    const data = encodeFunctionData({ abi, functionName, args });

    return estimateGas(config, { to, data });
  };

  const approve = async ({ amount, gas }: { amount: bigint; gas: bigint }) => {
    if (!address) throw new Error('Fungible token address is not defined');

    const functionName = FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE;
    const args = [ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS, amount] as const;

    return writeContractAsync({ address, abi, functionName, args, gas }).then(() => watch(amount));
  };

  return { ...useMutation({ mutationFn: approve }), getGasLimit };
}

export { useApprove };
