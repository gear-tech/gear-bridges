import { useMutation } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { estimateFeesPerGas } from 'wagmi/actions';

import { definedAssert } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues, UseHandleSubmitParameters } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';
import { usePermitUSDC } from './use-permit-usdc';
import { useTransfer } from './use-transfer';

const TRANSFER_GAS_LIMIT_FALLBACK = 21000n * 10n;

type Transaction = {
  call: () => Promise<unknown>;
  gasLimit: bigint;
  value?: bigint;
};

function useHandleEthSubmit({ fee, allowance, accountBalance, onTransactionStart }: UseHandleSubmitParameters) {
  const { token } = useBridgeContext();
  const isUSDC = token?.symbol.toLowerCase().includes('usdc');

  const mint = useMint();
  const approve = useApprove();
  const permitUSDC = usePermitUSDC();
  const transfer = useTransfer(fee);

  const config = useConfig();

  const getTransactions = async ({ amount, accountAddress }: FormattedValues) => {
    definedAssert(allowance, 'Allowance');
    definedAssert(fee, 'Fee');
    definedAssert(token, 'Fungible token');

    const txs: Transaction[] = [];
    const shouldMint = token.isNative;
    const shouldApprove = amount > allowance;

    if (shouldMint) {
      const value = amount;
      const gasLimit = await mint.getGasLimit(value);

      txs.push({
        call: () => mint.mutateAsync({ value }),
        gasLimit,
        value,
      });
    }

    // if approve is not made, transfer gas estimate will fail.
    // it can be avoided by using stateOverride,
    // but it requires the knowledge of the storage slot or state diff of the allowance for each token,
    // which is not feasible to do programmatically (at least I didn't managed to find a convenient way to do so).

    const bridgeTx = {
      gasLimit: shouldApprove ? TRANSFER_GAS_LIMIT_FALLBACK : await transfer.getGasLimit({ amount, accountAddress }),
      value: fee,
    };

    if (shouldApprove && isUSDC) {
      const call = () =>
        permitUSDC.mutateAsync(amount).then((permit) => transfer.mutateAsync({ amount, accountAddress, permit }));

      txs.push({ call, ...bridgeTx });

      return txs;
    }

    if (shouldApprove) {
      const call = () => approve.mutateAsync({ amount });
      const gasLimit = await approve.getGasLimit(amount);

      txs.push({ call, gasLimit });
    }

    const call = () => transfer.mutateAsync({ amount, accountAddress });
    txs.push({ call, ...bridgeTx });

    return txs;
  };

  const getRequiredBalance = async (values: FormattedValues) => {
    definedAssert(accountBalance, 'Account balance');
    definedAssert(fee, 'Fee value');

    const txs = await getTransactions(values);
    const { maxFeePerGas } = await estimateFeesPerGas(config);

    const totalGasLimit = txs.reduce((sum, { gasLimit }) => sum + gasLimit, 0n) * maxFeePerGas;
    const totalValue = txs.reduce((sum, { value }) => (value ? sum + value : sum), 0n);

    const requiredBalance = totalValue + totalGasLimit;
    const fees = totalGasLimit + fee;

    return { requiredBalance, fees };
  };

  const requiredBalance = useMutation({ mutationFn: getRequiredBalance });

  const resetState = () => {
    mint.reset();
    approve.reset();
    permitUSDC.reset();
    transfer.reset();
  };

  const onSubmit = async (values: FormattedValues) => {
    definedAssert(requiredBalance.data, 'Required balance');

    const txs = await getTransactions(values);

    resetState();
    onTransactionStart(values, requiredBalance.data.fees);

    for (const { call } of txs) await call();
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

  return { onSubmit: mutateAsync, isPending: isPending || requiredBalance.isPending, error, status, requiredBalance };
}

export { useHandleEthSubmit };
