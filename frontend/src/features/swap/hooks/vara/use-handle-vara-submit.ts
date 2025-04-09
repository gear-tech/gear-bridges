import { HexString } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { SubmittableExtrinsic } from '@polkadot/api/types';
import { ISubmittableResult } from '@polkadot/types/types';
import { useMutation } from '@tanstack/react-query';

import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { isUndefined } from '@/utils';

import { InsufficientAccountBalanceError } from '../../errors';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';
import { useSignAndSend } from './use-sign-and-send';
import { useTransfer, useTransferGasLimit } from './use-transfer';
import { useVFTManagerAddress } from './use-vft-manager-address';

function useHandleVaraSubmit(
  ftAddress: HexString | undefined,
  feeValue: bigint | undefined,
  allowance: bigint | undefined,
  ftBalance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTransactionModal: (amount: string, receiver: string) => void,
) {
  const { api, isApiReady } = useApi();
  const mint = useMint();
  const approve = useApprove(ftAddress);
  const transfer = useTransfer();
  const { data: vftManagerAddress, isLoading: isVftManagerAddressLoading } = useVFTManagerAddress();
  const { data: transferGasLimit, isLoading: isGasLimitLoading } = useTransferGasLimit();
  const isLoading = isVftManagerAddressLoading || isGasLimitLoading;
  const signAndSend = useSignAndSend();

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    if (!ftAddress) throw new Error('Fungible token address is not found');
    if (!vftManagerAddress) throw new Error('VFT manager address is not found');
    if (isUndefined(feeValue)) throw new Error('Fee is not found');
    if (isUndefined(allowance)) throw new Error('Allowance is not found');
    if (isUndefined(ftBalance)) throw new Error('FT balance is not found');
    if (isUndefined(transferGasLimit)) throw new Error('Gas limit is not found');
    if (!isApiReady) throw new Error('API is not initialized');
    if (isUndefined(accountBalance)) throw new Error('Account balance is not found');

    const isMintRequired = ftAddress === WRAPPED_VARA_CONTRACT_ADDRESS && amount > ftBalance;
    const valueToMint = isMintRequired ? amount - ftBalance : BigInt(0);

    const isApproveRequired = amount > allowance;

    const DEFAULT_TX = { transaction: undefined, awaited: { fee: BigInt(0) } };

    const preparedMint = isMintRequired
      ? await mint.prepareTransactionAsync({ args: [], value: valueToMint })
      : DEFAULT_TX;

    const preparedApprove = isApproveRequired
      ? await approve.prepareTransactionAsync({ args: [vftManagerAddress, amount] })
      : DEFAULT_TX;

    const preparedTransfer = await transfer.prepareTransactionAsync({
      gasLimit: transferGasLimit,
      args: [amount, accountAddress, ftAddress],
      value: feeValue,
    });

    // TODO: replace with calculated values after https://github.com/gear-tech/sails/issues/474 is resolved
    const mintGasLimit = BigInt(isMintRequired ? 20000000000 : 0);
    const approveGasLimit = BigInt(isApproveRequired ? 20000000000 : 0);

    const totalGasLimit = (mintGasLimit + approveGasLimit + transferGasLimit) * api.valuePerGas.toBigInt();
    const totalEstimatedFee = preparedMint.awaited.fee + preparedApprove.awaited.fee + preparedTransfer.awaited.fee;
    const requiredBalance =
      valueToMint + totalGasLimit + totalEstimatedFee + feeValue + api.existentialDeposit.toBigInt();

    if (accountBalance < requiredBalance) throw new InsufficientAccountBalanceError('VARA', requiredBalance);

    return {
      mintTx: preparedMint.transaction,
      approveTx: preparedApprove.transaction,
      transferTx: preparedTransfer.transaction,
    };
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    if (!isApiReady) throw new Error('API is not initialized');

    const { mintTx, approveTx, transferTx } = await validateBalance(amount, accountAddress);

    const extrinsics = [mintTx?.extrinsic, approveTx?.extrinsic, transferTx?.extrinsic].filter(
      Boolean,
    ) as SubmittableExtrinsic<'promise', ISubmittableResult>[];

    const extrinsic = api.tx.utility.batchAll(extrinsics);

    openTransactionModal(amount.toString(), accountAddress);

    return signAndSend.mutateAsync({ extrinsic });
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, { ...approve, isLoading }] as const;
}

export { useHandleVaraSubmit };
