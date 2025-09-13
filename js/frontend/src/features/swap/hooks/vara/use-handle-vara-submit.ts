import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';

import { definedAssert, isUndefined } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { Extrinsic, FormattedValues, UseHandleSubmitParameters } from '../../types';

import { usePayFeesWithAwait } from './use-pay-fees-with-await';
import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';
import { useSignAndSend } from './use-sign-and-send';
import { useVaraTxs } from './use-vara-txs';

function useHandleVaraSubmit({
  formValues,
  bridgingFee,
  shouldPayBridgingFee,
  vftManagerFee,
  allowance,
  onTransactionStart,
}: UseHandleSubmitParameters) {
  const { api } = useApi();

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();
  const payFees = usePayFeesWithAwait({ fee: bridgingFee, shouldPayBridgingFee });
  const signAndSend = useSignAndSend({ programs: [mint.program, approve.program, requestBridging.program] });

  const txs = useVaraTxs({ formValues, bridgingFee, shouldPayBridgingFee, vftManagerFee, allowance });

  const resetState = () => {
    signAndSend.reset();
    payFees.reset();
  };

  const sendTransactions = async (values: FormattedValues) => {
    definedAssert(api, 'API');
    definedAssert(txs.data, 'Prepared transactions');

    resetState();
    onTransactionStart(values);

    // event subscription to get nonce from bridging request reply, and send pay fee transaction.
    // since we're already checking replies in useSignAndSend,
    // would be nice to have the ability to decode it's payload there. perhaps some api in sails-js can be implemented?
    const { result, unsubscribe } = payFees.awaitBridgingRequest(values);
    let blockHash: HexString | undefined;

    try {
      const extrinsics = txs.data
        .map(({ extrinsic }) => extrinsic)
        .filter((extrinsic): extrinsic is Extrinsic => Boolean(extrinsic));

      const extrinsic = api.tx.utility.batchAll(extrinsics);
      const batchResult = await signAndSend.mutateAsync({ extrinsic });

      blockHash = batchResult.blockHash;
    } catch (error) {
      unsubscribe();
      throw error;
    }

    if (!blockHash) throw new Error('Block hash from request bridging message is not found');

    return result(blockHash);
  };

  const estimateTxs = () => {
    if (isUndefined(bridgingFee) || isUndefined(vftManagerFee) || !api || !txs.data) return;

    const totalGasLimit = txs.data.reduce((sum, { gasLimit }) => sum + gasLimit, 0n) * api.valuePerGas.toBigInt();
    const totalEstimatedFee = txs.data.reduce((sum, { estimatedFee }) => sum + estimatedFee, 0n);
    const totalValue = txs.data.reduce((sum, { value }) => (value ? sum + value : sum), 0n);

    const requiredBalance = totalGasLimit + totalEstimatedFee + totalValue + api.existentialDeposit.toBigInt();
    let fees = totalGasLimit + totalEstimatedFee + vftManagerFee;

    if (shouldPayBridgingFee) fees += bridgingFee;

    return { requiredBalance, fees };
  };

  const txsEstimate = estimateTxs();

  const getStatus = () => {
    if (signAndSend.isPending || signAndSend.error) return SUBMIT_STATUS.BRIDGE;
    if (payFees.isPending || payFees.error) return SUBMIT_STATUS.FEE;

    return SUBMIT_STATUS.SUCCESS;
  };

  const { mutateAsync, isPending, error } = useMutation({ mutationFn: sendTransactions });
  const status = getStatus();

  return { onSubmit: mutateAsync, isPending, error, status, txsEstimate };
}

export { useHandleVaraSubmit };
