import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { useMutation } from '@tanstack/react-query';

import { definedAssert } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { Extrinsic, FormattedValues } from '../../types';

import { usePayFeesWithAwait } from './use-pay-fees-with-await';
import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';
import { usePrepareVaraTxs } from './use-prepare-vara-txs';
import { useSignAndSend } from './use-sign-and-send';

type Params = {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  vftManagerFee: bigint | undefined;
  allowance: bigint | undefined;
  onTransactionStart: (values: FormattedValues) => void;
};

function useSendVaraTxs({ bridgingFee, shouldPayBridgingFee, vftManagerFee, allowance, onTransactionStart }: Params) {
  const { api } = useApi();

  const prepareVaraTxs = usePrepareVaraTxs({ bridgingFee, shouldPayBridgingFee, vftManagerFee, allowance });

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();

  const payFees = usePayFeesWithAwait({ fee: bridgingFee, shouldPayBridgingFee });
  const signAndSend = useSignAndSend({ programs: [mint.program, approve.program, requestBridging.program] });

  const resetState = () => {
    signAndSend.reset();
    payFees.reset();
  };

  const sendTxs = async (values: FormattedValues) => {
    definedAssert(api, 'API');
    definedAssert(prepareVaraTxs, 'Prepared transactions');

    const txs = await prepareVaraTxs(values);

    resetState();
    onTransactionStart(values);

    // event subscription to get nonce from bridging request reply, and send pay fee transaction.
    // since we're already checking replies in useSignAndSend,
    // would be nice to have the ability to decode it's payload there. perhaps some api in sails-js can be implemented?
    const { result, unsubscribe } = payFees.awaitBridgingRequest(values);
    let blockHash: HexString | undefined;

    try {
      const extrinsics = txs
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

  const getStatus = () => {
    if (signAndSend.isPending || signAndSend.error) return SUBMIT_STATUS.BRIDGE;
    if (payFees.isPending || payFees.error) return SUBMIT_STATUS.FEE;

    return SUBMIT_STATUS.SUCCESS;
  };

  return { ...useMutation({ mutationFn: sendTxs }), status: getStatus() };
}

export { useSendVaraTxs };
