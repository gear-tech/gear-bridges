import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { WatchContractEventOnLogsParameter } from 'viem';
import { useConfig, useWriteContract } from 'wagmi';
import { watchContractEvent } from 'wagmi/actions';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount } from '@/hooks';

import { EVENT_NAME } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';

import { useERC20ManagerAddress } from './use-erc20-manager-address';

const abi = FUNGIBLE_TOKEN_ABI;

function useApprove(address: HexString | undefined) {
  const ethAccount = useEthAccount();
  const config = useConfig();
  const { data: erc20ManagerAddress, isLoading } = useERC20ManagerAddress();
  const { writeContractAsync } = useWriteContract();

  // maybe better to use waitForTransactionReceipt,
  // but feels like it's getting fired before approval in contract
  const watch = (amount: bigint) =>
    new Promise<bigint>((resolve, reject) => {
      const eventName = EVENT_NAME.APPROVAL;
      const args = { owner: ethAccount.address, spender: erc20ManagerAddress };

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

  const approve = async (amount: bigint) => {
    if (!address) throw new Error('Fungible token address is not defined');
    if (!erc20ManagerAddress) throw new Error('ERC20 Manager address is not defined');

    const functionName = FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE;
    const args = [erc20ManagerAddress, amount] as const;

    return writeContractAsync({ address, abi, functionName, args }).then(() => watch(amount));
  };

  const mutation = useMutation({ mutationFn: approve });

  return { ...mutation, isLoading };
}

export { useApprove };
