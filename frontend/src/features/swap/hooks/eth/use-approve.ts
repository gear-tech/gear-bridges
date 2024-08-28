import { HexString } from '@gear-js/api';
import { useAlert } from '@gear-js/react-hooks';
import { BaseError, useConfig, useWriteContract } from 'wagmi';
import { watchContractEvent } from 'wagmi/actions';

import { useEthAccount, useLoading } from '@/hooks';
import { logger } from '@/utils';

import { FUNGIBLE_TOKEN_ABI, FUNCTION_NAME, EVENT_NAME, ERC20_TREASURY_CONTRACT_ADDRESS } from '../../consts';

function useApprove(address: HexString | undefined) {
  const alert = useAlert();
  const ethAccount = useEthAccount();
  const config = useConfig();
  const { writeContract } = useWriteContract();
  const [isPending, enablePending, disablePending] = useLoading();

  const abi = FUNGIBLE_TOKEN_ABI;

  const handleError = (message: string) => {
    disablePending();

    logger.error(FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE, new Error(message));
    alert.error(message);
  };

  const watch = (amount: bigint, onSuccess: () => void) => {
    const owner = ethAccount.address;
    const spender = ERC20_TREASURY_CONTRACT_ADDRESS;

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

    enablePending();

    logger.info(
      FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE,
      `\naddress: ${address}\nargs: [${ERC20_TREASURY_CONTRACT_ADDRESS}, ${amount}]`,
    );

    writeContract(
      {
        address,
        abi,
        functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE,
        args: [ERC20_TREASURY_CONTRACT_ADDRESS, amount],
      },
      {
        onSuccess: () => watch(amount, onSuccess),
        onError: (error) => handleError((error as BaseError).shortMessage || error.message),
      },
    );
  };

  return { write, isPending };
}

export { useApprove };
