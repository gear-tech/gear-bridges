import { HexString } from '@gear-js/api';
import { useApi, useAccount } from '@gear-js/react-hooks';

import { isUndefined } from '@/utils';

import { VftManagerProgram, CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { Extrinsic, FormattedValues } from '../../types';

import { usePreparePayPriorityFee } from './use-prepare-pay-priority-fee';
import { usePreparePayVaraFee } from './use-prepare-pay-vara-fee';
import { useSignAndSend } from './use-sign-and-send';

type BridgingRequestedEventData = Parameters<
  Parameters<VftManagerProgram['vftManager']['subscribeToBridgingRequestedEvent']>[0]
>[0];

type Params = {
  fee: bigint | undefined;
  priorityFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  shouldPayPriorityFee: boolean;
};

function usePayFeesWithAwait({ fee, priorityFee, shouldPayBridgingFee, shouldPayPriorityFee }: Params) {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();
  const { token } = useBridgeContext();

  const payFee = usePreparePayVaraFee();
  const payPriorityFees = usePreparePayPriorityFee();
  const signAndSend = useSignAndSend({ programs: [payFee.program, payPriorityFees.program] });

  const error = signAndSend.error || payFee.error || payPriorityFees.error;
  const isPending = signAndSend.isPending || payFee.isPending || payPriorityFees.isPending;

  const reset = () => {
    signAndSend.reset();
    payFee.reset();
    payPriorityFees.reset();
  };

  const payFees = ({ amount, accountAddress }: FormattedValues) => {
    if (!token?.address) throw new Error('Fungible token address is not found');
    if (isUndefined(fee)) throw new Error('Fee is not found');
    if (isUndefined(priorityFee)) throw new Error('Priority fee is not found');
    if (!account) throw new Error('Account is not found');
    if (!isApiReady) throw new Error('API is not initialized');

    if (!shouldPayBridgingFee && !shouldPayPriorityFee)
      return { result: () => Promise.resolve(undefined), unsubscribe: () => {} };

    const vftManagerProgram = new VftManagerProgram(api, CONTRACT_ADDRESS.VFT_MANAGER);
    let unsubscribe: (() => void) | undefined;

    const result = (requestBridgingBlockHash: HexString) =>
      new Promise((resolve, reject) => {
        const onEvent = async ({ nonce, sender, receiver, ...data }: BridgingRequestedEventData) => {
          if (
            data.vara_token_id !== token.address ||
            BigInt(data.amount) !== amount ||
            sender !== account.decodedAddress ||
            receiver !== accountAddress
          )
            return;

          try {
            const extrinsics: Extrinsic[] = [];

            if (shouldPayBridgingFee) {
              const { transaction } = await payFee.prepareTransactionAsync({
                args: [nonce],
                value: fee,
              });

              extrinsics.push(transaction.extrinsic);
            }

            if (shouldPayPriorityFee) {
              const { transaction } = await payPriorityFees.prepareTransactionAsync({
                args: [requestBridgingBlockHash, nonce],
                value: priorityFee,
                gasLimit: { increaseGas: 10 }, // may require higher gas after bridging fee payment is made, initial estimate done without it
              });

              extrinsics.push(transaction.extrinsic);
            }

            const extrinsic = api.tx.utility.batchAll(extrinsics);

            await signAndSend.mutateAsync({ extrinsic });
            resolve(undefined);
          } catch (err) {
            reject(err as Error);
          } finally {
            unsubscribe?.();
          }
        };

        vftManagerProgram.vftManager
          .subscribeToBridgingRequestedEvent(onEvent)
          .then((unsub) => (unsubscribe = unsub))
          .catch((err: Error) => reject(err));
      });

    return { result, unsubscribe: () => unsubscribe?.() };
  };

  return { awaitBridgingRequest: payFees, error, isPending, reset };
}

export { usePayFeesWithAwait };
