import { HexString } from '@gear-js/api';
import { useAlert } from '@gear-js/react-hooks';
import { BaseError, useConfig, useReadContract, useWriteContract } from 'wagmi';
import { watchContractEvent } from 'wagmi/actions';

import { FUNGIBLE_TOKEN_ABI } from '@/consts';
import { useEthAccount, useLoading } from '@/hooks';
import { logger } from '@/utils';

import { EVENT_NAME, BRIDGING_PAYMENT_ABI, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '../../consts';
import { FUNCTION_NAME } from '../../consts/eth';

const abi = FUNGIBLE_TOKEN_ABI;

function useERC20ManagerAddress() {
  return useReadContract({
    abi: BRIDGING_PAYMENT_ABI,
    address: ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS,
    functionName: 'getUnderlyingAddress',
  });
}

function useApprove(address: HexString | undefined) {
  const { data: erc20ManagerAddress, isLoading } = useERC20ManagerAddress();
  const { writeContract } = useWriteContract();
  const alert = useAlert();
  const ethAccount = useEthAccount();
  const config = useConfig();
  const [isPending, enablePending, disablePending] = useLoading();

  const handleError = (message: string) => {
    disablePending();

    logger.error(FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE, new Error(message));
    alert.error(message);
  };

  const watch = (amount: bigint, onSuccess: () => void) => {
    const owner = ethAccount.address;
    const spender = erc20ManagerAddress;

    // maybe better to use waitForTransactionReceipt,
    // but feels like it's getting fired before approval in contract
    const unwatch = watchContractEvent(config, {
      address,
      abi,
      eventName: EVENT_NAME.APPROVAL,
      args: { owner, spender },

      onLogs: (logs) =>
        logs.forEach(({ args: { value } }) => {
          disablePending();
          unwatch();

          if (!value || value < amount) return handleError('Approved value is less than the required amount');

          onSuccess();
        }),

      onError: ({ message }) => {
        unwatch();
        handleError(message);
      },
    });
  };

  const write = (amount: bigint, onSuccess: () => void) => {
    if (!address) throw new Error('Fungible token address is not defined');
    if (!erc20ManagerAddress) throw new Error('ERC20 Manager address is not defined');

    enablePending();

    writeContract(
      {
        address,
        abi,
        functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE,
        args: [erc20ManagerAddress, amount],
      },
      {
        onSuccess: () => watch(amount, onSuccess),
        onError: (error) => handleError((error as BaseError).shortMessage || error.message),
      },
    );
  };

  return { write, isPending, isLoading };
}

export { useApprove };
