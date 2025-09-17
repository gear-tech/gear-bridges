import { UseHandleSubmitParameters } from '../../types';

import { useEthTxsEstimate } from './use-eth-txs-estimate';
import { useSendEthTxs } from './use-send-eth-txs';

function useHandleEthSubmit({
  bridgingFee,
  shouldPayBridgingFee,
  formValues,
  accountBalance,
  onTransactionStart,
}: UseHandleSubmitParameters) {
  const sendEthTxs = useSendEthTxs({ bridgingFee, shouldPayBridgingFee, onTransactionStart });
  const { data: txsEstimate } = useEthTxsEstimate({ bridgingFee, shouldPayBridgingFee, formValues, accountBalance });

  return { ...sendEthTxs, txsEstimate };
}

export { useHandleEthSubmit };
