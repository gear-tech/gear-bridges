import { HexString } from '@gear-js/api';
import { useAccount, useApi } from '@gear-js/react-hooks';
import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';

import { VFT_MANAGER_CONTRACT_ADDRESS } from '@/consts';
import { useVaraSymbol } from '@/hooks';
import { definedAssert } from '@/utils';

import { useBridgeContext } from '../../context';
import { InsufficientAccountBalanceError } from '../../errors';
import { FormattedValues } from '../../types';

import { usePayFee } from './use-pay-fee';
import { usePrepareApprove } from './use-prepare-approve';
import { usePrepareMint } from './use-prepare-mint';
import { usePrepareRequestBridging } from './use-prepare-request-bridging';
import { useSignAndSend } from './use-sign-and-send';

function useHandleVaraSubmit(
  feeValue: bigint | undefined,
  allowance: bigint | undefined,
  ftBalance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTransactionModal: (amount: string, receiver: string) => void,
) {
  const { api } = useApi();
  const { account } = useAccount();
  const { token } = useBridgeContext();
  const varaSymbol = useVaraSymbol();

  const mint = usePrepareMint();
  const approve = usePrepareApprove();
  const requestBridging = usePrepareRequestBridging();

  const payFee = usePayFee(feeValue);
  const signAndSend = useSignAndSend();

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    definedAssert(api, 'API');
    definedAssert(varaSymbol, 'Vara symbol');
    definedAssert(token.address, 'Fungible token address');
    definedAssert(feeValue, 'Fee value');
    definedAssert(allowance, 'Allowance');
    definedAssert(ftBalance, 'Fungible token balance');
    definedAssert(accountBalance, 'Account balance');

    const isMintRequired = token.isNative && amount > ftBalance;
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
      args: [token.address, amount, accountAddress],
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

    if (accountBalance < requiredBalance) throw new InsufficientAccountBalanceError(varaSymbol, requiredBalance);

    return {
      mintTx: preparedMint.transaction,
      approveTx: preparedApprove.transaction,
      transferTx: preparedRequestBridging.transaction,
    };
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    definedAssert(api, 'API');
    definedAssert(account, 'Account');
    definedAssert(feeValue, 'Fee value');

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
