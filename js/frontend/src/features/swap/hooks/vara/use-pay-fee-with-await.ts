import { useApi, useAccount } from '@gear-js/react-hooks';

import { isUndefined } from '@/utils';

import { VftManagerProgram, CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

import { usePayVaraFee } from './use-pay-vara-fee';

type BridgingRequestedEventData = Parameters<
  Parameters<VftManagerProgram['vftManager']['subscribeToBridgingRequestedEvent']>[0]
>[0];

function usePayFeeWithAwait(feeValue: bigint | undefined) {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();
  const { token } = useBridgeContext();

  const { sendTransactionAsync, reset, error, isPending } = usePayVaraFee();

  const payFees = ({ amount, accountAddress }: FormattedValues) => {
    if (!token?.address) throw new Error('Fungible token address is not found');
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (!account) throw new Error('Account is not found');
    if (!isApiReady) throw new Error('API is not initialized');

    const vftManagerProgram = new VftManagerProgram(api, CONTRACT_ADDRESS.VFT_MANAGER);
    let unsubscribe: () => void | undefined;

    const result = new Promise((resolve, reject) => {
      const onEvent = ({ nonce, sender, receiver, ...data }: BridgingRequestedEventData) => {
        if (
          data.vara_token_id !== token.address ||
          BigInt(data.amount) !== amount ||
          sender !== account.decodedAddress ||
          receiver !== accountAddress
        )
          return;

        sendTransactionAsync({ args: [nonce], value: feeValue })
          .then(() => resolve(undefined))
          .catch((err: Error) => reject(err))
          .finally(() => unsubscribe());
      };

      vftManagerProgram.vftManager
        .subscribeToBridgingRequestedEvent(onEvent)
        .then((unsub) => (unsubscribe = unsub))
        .catch((err: Error) => reject(err));
    });

    return { result, unsubscribe: () => unsubscribe() };
  };

  return { awaitBridgingRequest: payFees, error, isPending, reset };
}

export { usePayFeeWithAwait };
