import { useAccount } from '@gear-js/react-hooks';
import { TransactionBuilder } from 'sails-js';

import { useLoading } from '@/hooks';
import { isUndefined } from '@/utils';

import { Options, useSignAndSend } from './use-sign-and-send';

function useSendMessage(gasLimit?: bigint) {
  const { account } = useAccount();
  const [isLoading, enableLoading, disableLoading] = useLoading();
  const signAndSend = useSignAndSend();

  return {
    sendMessage: async (transaction: TransactionBuilder<null>, value: bigint, args: Partial<Options>) => {
      if (isUndefined(account)) throw new Error('Account is not defined');
      enableLoading();

      const { signer } = account;
      transaction.withAccount(account.address, { signer });
      await transaction.withValue(value);

      if (gasLimit) {
        await transaction.withGas(gasLimit);
      } else {
        await transaction.calculateGas();
      }
      const { extrinsic } = transaction;

      const onSuccess = () => {
        args.onSuccess?.();
        disableLoading();
      };

      const onError = () => {
        args.onError?.();
        disableLoading();
      };

      const onFinally = () => {
        disableLoading();
      };

      signAndSend(extrinsic, { ...args, onSuccess, onError, onFinally });
    },

    isPending: isLoading,
  };
}

export { useSendMessage };
