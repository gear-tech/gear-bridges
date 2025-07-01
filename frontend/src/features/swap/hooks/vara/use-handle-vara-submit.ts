import { useApi } from '@gear-js/react-hooks';
import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';

import { definedAssert } from '@/utils';

import { SUBMIT_STATUS, CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues, UseHandleSubmitParameters } from '../../types';

import { usePayFee } from './use-pay-fee';
import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';
import { useSignAndSend } from './use-sign-and-send';

type Extrinsic = SubmittableExtrinsic<'promise', ISubmittableResult>;

type Transaction = {
  extrinsic: Extrinsic | undefined;
  gasLimit: bigint;
  estimatedFee: bigint;
  value?: bigint;
};

const GAS_LIMIT = {
  BRIDGE: 150_000_000_000n,
  APPROXIMATE_PAY_FEE: 10_000_000_000n,
} as const;

function useHandleVaraSubmit({ bridgingFee, vftManagerFee, allowance, onTransactionStart }: UseHandleSubmitParameters) {
  const { api } = useApi();
  const { token } = useBridgeContext();

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();
  const payFee = usePayFee(bridgingFee);
  const signAndSend = useSignAndSend({ programs: [mint.program, approve.program, requestBridging.program] });

  const getTransactions = async ({ amount, accountAddress }: FormattedValues) => {
    definedAssert(allowance, 'Allowance');
    definedAssert(bridgingFee, 'Bridging fee value');
    definedAssert(vftManagerFee, 'VFT Manager fee value');
    definedAssert(token, 'Fungible token');

    const txs: Transaction[] = [];
    const shouldMint = token.isNative;
    const shouldApprove = amount > allowance;

    if (shouldMint) {
      const { transaction, awaited } = await mint.prepareTransactionAsync({ args: [], value: amount });

      txs.push({
        extrinsic: transaction.extrinsic,
        gasLimit: transaction.gasInfo.min_limit.toBigInt(),
        estimatedFee: awaited.fee,
        value: amount,
      });
    }

    if (shouldApprove) {
      const { transaction, awaited } = await approve.prepareTransactionAsync({
        args: [CONTRACT_ADDRESS.VFT_MANAGER, amount],
      });

      txs.push({
        extrinsic: transaction.extrinsic,
        gasLimit: transaction.gasInfo.min_limit.toBigInt(),
        estimatedFee: awaited.fee,
      });
    }

    const { transaction, awaited } = await requestBridging.prepareTransactionAsync({
      gasLimit: GAS_LIMIT.BRIDGE,
      args: [token.address, amount, accountAddress],
    });

    txs.push({
      extrinsic: transaction.extrinsic,
      gasLimit: GAS_LIMIT.BRIDGE,
      estimatedFee: awaited.fee,
      value: vftManagerFee,
    });

    txs.push({
      extrinsic: undefined,
      gasLimit: GAS_LIMIT.APPROXIMATE_PAY_FEE,
      estimatedFee: awaited.fee, // cuz we don't know payFees gas limit yet
      value: bridgingFee,
    });

    return txs;
  };

  const resetState = () => {
    signAndSend.reset();
    payFee.reset();
  };

  const sendTransactions = async (values: FormattedValues) => {
    definedAssert(api, 'API');
    definedAssert(requiredBalance.data, 'Required balance');

    const txs = await getTransactions(values);

    resetState();
    onTransactionStart(values, requiredBalance.data.fees);

    // event subscription to get nonce from bridging request reply, and send pay fee transaction.
    // since we're already checking replies in useSignAndSend,
    // would be nice to have the ability to decode it's payload there. perhaps some api in sails-js can be implemented?
    const { result, unsubscribe } = payFee.awaitBridgingRequest(values);

    try {
      const extrinsics = txs
        .map(({ extrinsic }) => extrinsic)
        .filter((extrinsic): extrinsic is Extrinsic => Boolean(extrinsic));

      const extrinsic = api.tx.utility.batchAll(extrinsics);

      await signAndSend.mutateAsync({ extrinsic });
    } catch (error) {
      unsubscribe();
      throw error;
    }

    return result;
  };

  const getRequiredBalance = async (values: FormattedValues) => {
    definedAssert(api, 'API');
    definedAssert(bridgingFee, 'Fee value');
    definedAssert(vftManagerFee, 'VFT Manager fee value');

    const txs = await getTransactions(values);

    const totalGasLimit = txs.reduce((sum, { gasLimit }) => sum + gasLimit, 0n) * api.valuePerGas.toBigInt();
    const totalEstimatedFee = txs.reduce((sum, { estimatedFee }) => sum + estimatedFee, 0n);
    const totalValue = txs.reduce((sum, { value }) => (value ? sum + value : sum), 0n);

    const requiredBalance = totalGasLimit + totalEstimatedFee + totalValue + api.existentialDeposit.toBigInt();
    const fees = totalGasLimit + totalEstimatedFee + bridgingFee + vftManagerFee;

    return { requiredBalance, fees };
  };

  const requiredBalance = useMutation({ mutationFn: getRequiredBalance });

  const getStatus = () => {
    if (signAndSend.isPending || signAndSend.error) return SUBMIT_STATUS.BRIDGE;
    if (payFee.isPending || payFee.error) return SUBMIT_STATUS.FEE;

    return SUBMIT_STATUS.SUCCESS;
  };

  const { mutateAsync, isPending, error } = useMutation({ mutationFn: sendTransactions });
  const status = getStatus();

  return { onSubmit: mutateAsync, isPending: isPending || requiredBalance.isPending, error, status, requiredBalance };
}

export { useHandleVaraSubmit };
