import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';

import { VFT_MANAGER_CONTRACT_ADDRESS, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { isUndefined } from '@/utils';

import { InsufficientAccountBalanceError } from '../../errors';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';
import { usePayFee } from './use-pay-fee';
import { useRequestBridging } from './use-request-bridging';
import { useSignAndSend } from './use-sign-and-send';

function useHandleVaraSubmit(
  ftAddress: HexString | undefined,
  feeValue: bigint | undefined,
  allowance: bigint | undefined,
  ftBalance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTransactionModal: (amount: string, receiver: string) => void,
) {
  const { api, isApiReady } = useApi();
  const { account } = useAccount();
  const mint = useMint();
  const approve = useApprove(ftAddress);
  const requestBridging = useRequestBridging();
  const payFee = usePayFee(ftAddress, feeValue);
  const signAndSend = useSignAndSend();

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (isUndefined(allowance)) throw new Error('Allowance is not found');
    if (isUndefined(ftBalance)) throw new Error('FT balance is not found');
    if (!isApiReady) throw new Error('API is not initialized');
    if (isUndefined(accountBalance)) throw new Error('Account balance is not found');

    const isMintRequired = ftAddress === WRAPPED_VARA_CONTRACT_ADDRESS && amount > ftBalance;
    const valueToMint = isMintRequired ? amount - ftBalance : BigInt(0);
    const isApproveRequired = amount > allowance;
    const DEFAULT_TX = { transaction: undefined, awaited: { fee: BigInt(0) } };
    const maxGasLimit = api.blockGasLimit.toBigInt();

    const preparedMint = isMintRequired
      ? await mint.prepareTransactionAsync({ args: [], value: valueToMint })
      : DEFAULT_TX;

    const preparedApprove = isApproveRequired
      ? await approve.prepareTransactionAsync({ args: [VFT_MANAGER_CONTRACT_ADDRESS, amount] })
      : DEFAULT_TX;

    const preparedRequestBridging = await requestBridging.prepareTransactionAsync({
      gasLimit: maxGasLimit,
      args: [ftAddress, amount, accountAddress],
    });

    // TODO: replace with calculated values after https://github.com/gear-tech/sails/issues/474 is resolved
    const mintGasLimit = BigInt(isMintRequired ? 20000000000 : 0);
    const approveGasLimit = BigInt(isApproveRequired ? 20000000000 : 0);

    // cuz we don't know payFees gas limit yet
    const transferGasLimit = maxGasLimit * 2n;
    const transferEstimatedFee = preparedRequestBridging.awaited.fee * 2n;

    const totalGasLimit = (mintGasLimit + approveGasLimit + transferGasLimit) * api.valuePerGas.toBigInt();
    const totalEstimatedFee = preparedMint.awaited.fee + preparedApprove.awaited.fee + transferEstimatedFee;
    const requiredBalance =
      valueToMint + totalGasLimit + totalEstimatedFee + feeValue + api.existentialDeposit.toBigInt();

    if (accountBalance < requiredBalance) throw new InsufficientAccountBalanceError('VARA', requiredBalance);

    return {
      mintTx: preparedMint.transaction,
      approveTx: preparedApprove.transaction,
      transferTx: preparedRequestBridging.transaction,
    };
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (!account) throw new Error('Account is not found');
    if (!isApiReady) throw new Error('API is not initialized');

    const { mintTx, approveTx, transferTx } = await validateBalance(amount, accountAddress);

    const extrinsics = [mintTx?.extrinsic, approveTx?.extrinsic, transferTx?.extrinsic].filter(
      Boolean,
    ) as SubmittableExtrinsic<'promise', ISubmittableResult>[];

    const { result, unsubscribe } = payFee.awaitBridgingRequest({ amount, accountAddress });

    openTransactionModal(amount.toString(), accountAddress);

    const extrinsic = api.tx.utility.batchAll(extrinsics);

    try {
      await signAndSend.mutateAsync({ extrinsic });
    } catch (error) {
      unsubscribe();
      throw error;
    }

    return result;
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, approve, payFee] as const;
}

export { useHandleVaraSubmit };
