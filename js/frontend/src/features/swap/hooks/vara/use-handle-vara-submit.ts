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
  const sendTxs = useSendVaraTxs({ bridgingFee, shouldPayBridgingFee, vftManagerFee, allowance, onTransactionStart });

  const { data: txsEstimate } = useVaraTxsEstimate({
    formValues,
    bridgingFee,
    shouldPayBridgingFee,
    vftManagerFee,
    allowance,
  });

  return { ...sendTxs, txsEstimate };
}

export { useHandleVaraSubmit };
