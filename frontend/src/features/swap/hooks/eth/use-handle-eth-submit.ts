import { HexString } from '@gear-js/api';
import { useMutation } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { estimateFeesPerGas } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { useBridgeContext } from '../../context';
import { InsufficientAccountBalanceError } from '../../errors';
import { FormattedValues, UseHandleSubmitParameters } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';
import { usePermitUSDC } from './use-permit-usdc';
import { useTransfer } from './use-transfer';

const TRANSFER_GAS_LIMIT_FALLBACK = 21000n * 10n;

function useHandleEthSubmit({ fee, allowance, accountBalance, onTransactionStart }: UseHandleSubmitParameters) {
  const { token } = useBridgeContext();
  const isUSDC = token?.symbol.toLowerCase().includes('usdc');

  const mint = useMint();
  const approve = useApprove();
  const permitUSDC = usePermitUSDC();
  const transfer = useTransfer(fee);

  const config = useConfig();

  const validateBalance = async (amount: bigint, accountAddress: HexString) => {
    definedAssert(token, 'Fungible token');
    definedAssert(fee, 'Fee');
    definedAssert(allowance, 'Allowance');
    definedAssert(accountBalance, 'Account balance');

    const valueToMint = token.isNative ? amount : 0n;
    const isMintRequired = valueToMint > 0n;
    const mintGasLimit = isMintRequired ? await mint.getGasLimit(valueToMint) : 0n;

    const isApproveRequired = amount > allowance;
    const approveGasLimit = isApproveRequired && !isUSDC ? await approve.getGasLimit(amount) : 0n;

    // if approve is not made, transfer gas estimate will fail.
    // it can be avoided by using stateOverride,
    // but it requires the knowledge of the storage slot or state diff of the allowance for each token,
    // which is not feasible to do programmatically (at least I didn't managed to find a convenient way to do so).
    const transferGasLimit = isApproveRequired ? undefined : await transfer.getGasLimit({ amount, accountAddress });

    // TRANSFER_GAS_LIMIT_FALLBACK is just for balance check, during the actual transfer it will be recalculated
    const gasLimit = mintGasLimit + approveGasLimit + (transferGasLimit || TRANSFER_GAS_LIMIT_FALLBACK);

    const { maxFeePerGas } = await estimateFeesPerGas(config);
    const weiGasLimit = gasLimit * maxFeePerGas;

    const balanceToWithdraw = valueToMint + weiGasLimit + fee;

    if (balanceToWithdraw > accountBalance) throw new InsufficientAccountBalanceError('ETH', balanceToWithdraw);

    return { isMintRequired, valueToMint, isApproveRequired };
  };

  const onSubmit = async ({ amount, accountAddress }: FormattedValues) => {
    const { isMintRequired, valueToMint, isApproveRequired } = await validateBalance(amount, accountAddress);

    mint.reset();
    approve.reset();
    permitUSDC.reset();
    transfer.reset();

    onTransactionStart(amount, accountAddress);

    if (isMintRequired) await mint.mutateAsync({ value: valueToMint });

    if (isApproveRequired && isUSDC) {
      const permit = await permitUSDC.mutateAsync(amount);

      return transfer.mutateAsync({ amount, accountAddress, permit });
    }

    if (isApproveRequired) await approve.mutateAsync({ amount });

    return transfer.mutateAsync({ amount, accountAddress });
  };

  const getStatus = () => {
    if (mint.isPending || mint.error) return SUBMIT_STATUS.MINT;
    if (approve.isPending || approve.error) return SUBMIT_STATUS.APPROVE;
    if (permitUSDC.isPending || permitUSDC.error) return SUBMIT_STATUS.PERMIT;
    if (transfer.isPending || transfer.error) return SUBMIT_STATUS.BRIDGE;

    return SUBMIT_STATUS.SUCCESS;
  };

  const { mutateAsync, isPending, error } = useMutation({ mutationFn: onSubmit });
  const status = getStatus();

  return { onSubmit: mutateAsync, isPending, error, status };
}

export { useHandleEthSubmit };
