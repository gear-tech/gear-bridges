import { HexString } from '@gear-js/api';
import { useAlert } from '@gear-js/react-hooks';
import { BaseError, useConfig, useWriteContract } from 'wagmi';
import { watchContractEvent } from 'wagmi/actions';

import { useEthAccount, useLoading } from '@/hooks';
import { logger } from '@/utils';

import { FUNGIBLE_TOKEN_ABI, FUNCTION_NAME, EVENT_NAME } from '../../consts';

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

  const watch = (bridgeAddress: HexString, amount: bigint, onSuccess: () => void) => {
    const owner = ethAccount.address;
    const spender = bridgeAddress;

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

  const write = (bridgeAddress: HexString, amount: bigint, onSuccess: () => void) => {
    if (!address) throw new Error('Fungible token address is not defined');

    enablePending();
    logger.info(FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE, `\naddress: ${address}\nargs: [${bridgeAddress}, ${amount}]`);

    writeContract(
      {
        address,
        abi,
        functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_APPROVE,
        args: [bridgeAddress, amount],
      },
      {
        onSuccess: () => watch(bridgeAddress, amount, onSuccess),
        onError: (error) => handleError((error as BaseError).shortMessage || error.message),
      },
    );
  };

  return { write, isPending };
}

export { useApprove };
