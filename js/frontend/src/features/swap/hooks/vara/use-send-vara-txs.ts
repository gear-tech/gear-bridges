import { useApi } from '@gear-js/react-hooks';
import { u8aToHex } from '@polkadot/util';
import { useMutation } from '@tanstack/react-query';
import { getServiceNamePrefix, getFnNamePrefix } from 'sails-js';

import { definedAssert } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { useBridgeContext } from '../../context';
import { assertBridgeReply, submitVaraBridgeRequest, type BridgeReply } from '../../submission';
import { Extrinsic, FormattedValues } from '../../types';

import { usePayFees } from './use-pay-fees';
import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';
import { usePrepareVaraTxs } from './use-prepare-vara-txs';
import { useSignAndSend } from './use-sign-and-send';

type Params = {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  vftManagerFee: bigint | undefined;
  onTransactionStart: (values: FormattedValues) => void;
};

function useSendVaraTxs({ bridgingFee, shouldPayBridgingFee, vftManagerFee, onTransactionStart }: Params) {
  const { api } = useApi();
  const { token } = useBridgeContext();

  const prepareTxs = usePrepareVaraTxs({
    bridgingFee,
    shouldPayBridgingFee,
    vftManagerFee,
  });

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();

  const payFees = usePayFees({ fee: bridgingFee, shouldPayBridgingFee });
  const signAndSend = useSignAndSend({ programs: [mint.program, approve.program, requestBridging.program] });

  const resetState = () => {
    signAndSend.reset();
    payFees.reset();
  };

  const sendTxs = async (values: FormattedValues) => {
    definedAssert(api, 'API');
    definedAssert(prepareTxs, 'Prepared transactions');

    const txs = await prepareTxs(values);

    resetState();
    onTransactionStart(values);

    const destinationAddress = token?.destinationAddress;
    definedAssert(destinationAddress, 'Mapped destination token');
    const transactions = txs.filter((tx) => tx.purpose !== undefined);
    const extrinsics = transactions.map(({ extrinsic }) => {
      definedAssert(extrinsic, 'Prepared extrinsic');
      return extrinsic as Extrinsic;
    });
    return submitVaraBridgeRequest(
      async () => {
        const original = await signAndSend.mutateAsync({
          extrinsic: api.tx.utility.batchAll(extrinsics),
          destinations: transactions.map(({ programId }) => programId),
        });

        let bridge: BridgeReply | undefined;
        let messageId: string | undefined;
        try {
          for (const [index, transaction] of transactions.entries()) {
            const reply = original.replies[index];
            const payload = u8aToHex(reply.payload);
            if (transaction.purpose === 'bridge') {
              if (
                bridge ||
                getServiceNamePrefix(payload) !== 'VftManager' ||
                getFnNamePrefix(payload) !== 'RequestBridging'
              )
                throw new Error('Expected exactly one original VftManager.RequestBridging reply');
              bridge = assertBridgeReply(transaction.decodePayload(reply.payload), destinationAddress);
              messageId = reply.messageId;
            } else if (transaction.purpose === 'approve') {
              if (
                getServiceNamePrefix(payload) !== 'Vft' ||
                getFnNamePrefix(payload) !== 'Approve' ||
                typeof transaction.decodePayload(reply.payload) !== 'boolean'
              )
                throw new Error('Malformed approval reply');
              // False can mean an unchanged allowance. The successful original bridge
              // reply below proves the manager could spend the required amount.
            } else {
              if (
                reply.payload.length &&
                (getServiceNamePrefix(payload) !== 'VftNativeExchange' || getFnNamePrefix(payload) !== 'Mint')
              )
                throw new Error('Unexpected native mint reply');
              if (transaction.decodePayload(reply.payload) !== null) throw new Error('Unexpected native mint result');
            }
          }
          if (!bridge) throw new Error('Original bridge reply is missing');
        } catch (error) {
          throw new Error(
            `Source transaction ${original.txHash}, messages ${original.replies.map((reply) => reply.messageId).join(', ')}: ${error instanceof Error ? error.message : String(error)}`,
          );
        }
        return { ...original, ...bridge, messageId };
      },
      shouldPayBridgingFee,
      payFees.payFees,
    );
  };

  const getStatus = () => {
    if (signAndSend.isPending || signAndSend.error) return SUBMIT_STATUS.BRIDGE;
    if (payFees.isPending || payFees.error) return SUBMIT_STATUS.FEE;

    return SUBMIT_STATUS.SUCCESS;
  };

  return { ...useMutation({ mutationFn: sendTxs }), status: getStatus() };
}

export { useSendVaraTxs };
