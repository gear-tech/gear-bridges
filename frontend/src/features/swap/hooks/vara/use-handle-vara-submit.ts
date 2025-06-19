import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';

import { useVaraSymbol } from '@/hooks';
import { definedAssert } from '@/utils';

import { SUBMIT_STATUS, CONTRACT_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { InsufficientAccountBalanceError } from '../../errors';
import { FormattedValues, UseHandleSubmitParameters } from '../../types';

import { usePayFee } from './use-pay-fee';
import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';
import { useSignAndSend } from './use-sign-and-send';

type Extrinsic = SubmittableExtrinsic<'promise', ISubmittableResult>;

const DEFAULT_TX = { transaction: undefined, awaited: { fee: BigInt(0) } };
const BRIDGING_REQUEST_GAS_LIMIT = 150_000_000_000n;
const APPROXIMATE_PAY_FEE_GAS_LIMIT = 10_000_000_000n;

function useHandleVaraSubmit({ fee, allowance, accountBalance, onTransactionStart }: UseHandleSubmitParameters) {
  const { api } = useApi();
  const { token } = useBridgeContext();
  const varaSymbol = useVaraSymbol();

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();
  const payFee = usePayFee(fee);
  const signAndSend = useSignAndSend({ programs: [mint.program, approve.program, requestBridging.program] });

  const getTransactions = async (amount: bigint, accountAddress: HexString) => {
    definedAssert(token, 'Fungible token');
    definedAssert(allowance, 'Allowance');

    const valueToMint = token.isNative ? amount : 0n;
    const isApproveRequired = amount > allowance;

    const preparedMint =
      valueToMint > 0n ? await mint.prepareTransactionAsync({ args: [], value: valueToMint }) : DEFAULT_TX;

    const preparedApprove = isApproveRequired
      ? await approve.prepareTransactionAsync({ args: [CONTRACT_ADDRESS.VFT_MANAGER, amount] })
      : DEFAULT_TX;

    const preparedRequestBridging = await requestBridging.prepareTransactionAsync({
      gasLimit: BRIDGING_REQUEST_GAS_LIMIT,
      args: [token.address, amount, accountAddress],
    });

    return { valueToMint, isApproveRequired, preparedMint, preparedApprove, preparedRequestBridging };
  };

  const getRequiredBalance = ({
    valueToMint,
    preparedMint,
    preparedApprove,
    preparedRequestBridging,
  }: Awaited<ReturnType<typeof getTransactions>>) => {
    definedAssert(api, 'API');
    definedAssert(fee, 'Fee value');

    const mintGasLimit = preparedMint.transaction?.gasInfo.min_limit.toBigInt() ?? 0n;
    const approveGasLimit = preparedApprove.transaction?.gasInfo.min_limit.toBigInt() ?? 0n;
    const estimatedBridgingFee = preparedRequestBridging.awaited.fee * 2n; // cuz we don't know payFees gas limit yet

    const totalGasLimit =
      (mintGasLimit + approveGasLimit + BRIDGING_REQUEST_GAS_LIMIT + APPROXIMATE_PAY_FEE_GAS_LIMIT) *
      api.valuePerGas.toBigInt();

    const totalEstimatedFee = preparedMint.awaited.fee + preparedApprove.awaited.fee + estimatedBridgingFee;

    return valueToMint + totalGasLimit + totalEstimatedFee + fee + api.existentialDeposit.toBigInt();
  };

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    definedAssert(varaSymbol, 'Vara symbol');
    definedAssert(accountBalance, 'Account balance');

    const transactions = await getTransactions(amount, accountAddress);
    const { preparedMint, preparedApprove, preparedRequestBridging } = transactions;

    const requiredBalance = getRequiredBalance(transactions);

    if (accountBalance < requiredBalance) throw new InsufficientAccountBalanceError(varaSymbol, requiredBalance);

    return {
      mintTx: preparedMint.transaction,
      approveTx: preparedApprove.transaction,
      transferTx: preparedRequestBridging.transaction,
    };
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    definedAssert(api, 'API');

    const { mintTx, approveTx, transferTx } = await validateBalance(amount, accountAddress);

    // event subscription to get nonce from bridging request reply, and send pay fee transaction.
    // since we're already checking replies in useSignAndSend,
    // would be nice to have the ability to decode it's payload there. perhaps some api in sails-js can be implemented?
    const { result, unsubscribe } = payFee.awaitBridgingRequest({ amount, accountAddress });

    signAndSend.reset();
    payFee.reset();

    onTransactionStart(amount, accountAddress);

    const extrinsics = [mintTx?.extrinsic, approveTx?.extrinsic, transferTx.extrinsic].filter(Boolean) as Extrinsic[];
    const extrinsic = api.tx.utility.batchAll(extrinsics);

    try {
      await signAndSend.mutateAsync({ extrinsic });
    } catch (error) {
      unsubscribe();
      throw error;
    }

    return result;
  };

  const getStatus = () => {
    if (signAndSend.isPending || signAndSend.error) return SUBMIT_STATUS.BRIDGE;
    if (payFee.isPending || payFee.error) return SUBMIT_STATUS.FEE;

    return SUBMIT_STATUS.SUCCESS;
  };

  const { mutateAsync, isPending, error } = useMutation({ mutationFn: onSubmit });
  const status = getStatus();

  return { onSubmit: mutateAsync, isPending, error, status };
}

export { useHandleVaraSubmit };
