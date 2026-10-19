import { useApi } from '@gear-js/react-hooks';

import { definedAssert } from '@/utils';

import { usePreparePayVaraFee } from './use-prepare-pay-vara-fee';
import { useSignAndSend } from './use-sign-and-send';

type Params = {
  fee: bigint | undefined;
  shouldPayBridgingFee: boolean;
};

function usePayFees({ fee, shouldPayBridgingFee }: Params) {
  const { api } = useApi();

  const payFee = usePreparePayVaraFee();
  const signAndSend = useSignAndSend({ programs: [payFee.program] });

  const error = signAndSend.error || payFee.error;
  const isPending = signAndSend.isPending || payFee.isPending;

  const reset = () => {
    signAndSend.reset();
    payFee.reset();
  };

  const payFees = async ({ nonce }: { nonce: bigint }): Promise<void> => {
    if (!shouldPayBridgingFee) return;
    definedAssert(api, 'API');
    definedAssert(fee, 'Fee');

    const { transaction } = await payFee.prepareTransactionAsync({ args: [nonce], value: fee });
    await signAndSend.mutateAsync({
      extrinsic: api.tx.utility.batchAll([transaction.extrinsic]),
      destinations: [transaction.programId],
    });
  };

  return { payFees, error, isPending, reset };
}

export { usePayFees };
