import { UseHandleSubmitParameters } from '../../types';

import { useEthTxsEstimate } from './use-eth-txs-estimate';
import { useSendEthTxs } from './use-send-eth-txs';

function useHandleEthSubmit({
  bridgingFee,
  shouldPayBridgingFee,
  allowance,
  formValues,
  onTransactionStart,
}: UseHandleSubmitParameters) {
  const sendEthTxs = useSendEthTxs({ allowance, bridgingFee, shouldPayBridgingFee, onTransactionStart });
  const { data: txsEstimate } = useEthTxsEstimate({ allowance, bridgingFee, shouldPayBridgingFee, formValues });

  return { ...sendEthTxs, txsEstimate };
}

export { useHandleEthSubmit };
