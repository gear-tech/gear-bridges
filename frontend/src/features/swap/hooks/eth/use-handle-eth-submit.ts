import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { estimateFeesPerGas } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { useBridgeContext } from '../../context';
import { InsufficientAccountBalanceError } from '../../errors';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';
import { useTransfer } from './use-transfer';

const TRANSFER_GAS_LIMIT_FALLBACK = 21000n * 10n;

function useHandleEthSubmit(
  fee: bigint | undefined,
  allowance: bigint | undefined,
  ftBalance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTransactionModal: (amount: string, receiver: string) => void,
) {
  const { token } = useBridgeContext();
  const mint = useMint();
  const approve = useApprove();
  const transfer = useTransfer(fee);
  const config = useConfig();

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    definedAssert(token?.address, 'Fungible token address');
    definedAssert(fee, 'Fee');
    definedAssert(allowance, 'Allowance');
    definedAssert(ftBalance, 'Fungible token balance');
    definedAssert(accountBalance, 'Account balance');

    const valueToMint = token.isNative ? amount : 0n;
    const isMintRequired = valueToMint > 0n;
    const mintGasLimit = isMintRequired ? await mint.getGasLimit(valueToMint) : 0n;

    const isApproveRequired = amount > allowance;
    const approveGasLimit = isApproveRequired ? await approve.getGasLimit(amount) : 0n;

    // if approve is not made, transfer gas estimate will fail.
    // it can be avoided by using stateOverride,
    // but it requires the knowledge of the storage slot or state diff of the allowance for each token,
    // which is not feasible to do programmatically (at least I didn't managed to find a convenient way to do so).
    const transferGasLimit = isApproveRequired ? undefined : await transfer.getGasLimit(amount, accountAddress);

    // TRANSFER_GAS_LIMIT_FALLBACK is just for balance check, during the actual transfer it will be recalculated
    const gasLimit = mintGasLimit + approveGasLimit + (transferGasLimit || TRANSFER_GAS_LIMIT_FALLBACK);

    const { maxFeePerGas } = await estimateFeesPerGas(config);
    const weiGasLimit = gasLimit * maxFeePerGas;

    const balanceToWithdraw = valueToMint + weiGasLimit + fee;

    if (balanceToWithdraw > accountBalance) throw new InsufficientAccountBalanceError('ETH', balanceToWithdraw);

    return { isMintRequired, valueToMint, isApproveRequired, mintGasLimit, approveGasLimit, transferGasLimit };
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    const { isMintRequired, valueToMint, isApproveRequired, mintGasLimit, approveGasLimit, transferGasLimit } =
      await validateBalance(amount, accountAddress);

    openTransactionModal(amount.toString(), accountAddress);

    if (isMintRequired) {
      await mint.mutateAsync({ value: valueToMint, gas: mintGasLimit });
    } else {
      mint.reset();
    }

    if (isApproveRequired) {
      await approve.mutateAsync({ amount, gas: approveGasLimit });
    } else {
      approve.reset();
    }

    return transfer.mutateAsync(amount, accountAddress, transferGasLimit);
  };

  const submit = useMutation({ mutationFn: onSubmit });

  return [submit, approve, undefined, mint] as const;
}

export { useHandleEthSubmit };
