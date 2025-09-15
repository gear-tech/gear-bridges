import { UseHandleSubmitParameters } from '../../types';

import { useSendVaraTxs } from './use-send-vara-txs';
import { useVaraTxsEstimate } from './use-vara-txs-estimate';

function useHandleVaraSubmit({
  formValues,
  bridgingFee,
  shouldPayBridgingFee,
  vftManagerFee,
  allowance,
  onTransactionStart,
}: UseHandleSubmitParameters) {
  const { mutateAsync, isPending, error, status } = useSendVaraTxs({
    bridgingFee,
    shouldPayBridgingFee,
    vftManagerFee,
    allowance,
    onTransactionStart,
  });

  const { data: txsEstimate } = useVaraTxsEstimate({
    formValues,
    bridgingFee,
    shouldPayBridgingFee,
    vftManagerFee,
    allowance,
  });

  return { onSubmit: mutateAsync, isPending, error, status, txsEstimate };
}

export { useHandleVaraSubmit };
