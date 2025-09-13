import { useMutation } from '@tanstack/react-query';
import { useEstimateFeesPerGas } from 'wagmi';

import { definedAssert, isUndefined } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { FormattedValues, UseHandleSubmitParameters } from '../../types';

import { useApprove } from './use-approve';
import { useEthTxs } from './use-eth-txs';
import { useMint } from './use-mint';
import { usePermitUSDC } from './use-permit-usdc';
import { useTransfer } from './use-transfer';

function useHandleEthSubmit({
  bridgingFee,
  shouldPayBridgingFee,
  allowance,
  formValues,
  onTransactionStart,
}: UseHandleSubmitParameters) {
  const mint = useMint();
  const approve = useApprove();
  const permitUSDC = usePermitUSDC();

  const { transferWithoutFee, transferWithFee } = useTransfer(bridgingFee);
  const transfer = shouldPayBridgingFee ? transferWithFee : transferWithoutFee;

  const txs = useEthTxs({ allowance, bridgingFee, shouldPayBridgingFee, formValues });

  const { data: feesPerGas } = useEstimateFeesPerGas();

  const estimateTxs = () => {
    if (isUndefined(bridgingFee) || isUndefined(feesPerGas) || !txs.data) return;

    const { maxFeePerGas } = feesPerGas;

    const totalGasLimit = txs.data.reduce((sum, { gasLimit }) => sum + gasLimit, 0n) * maxFeePerGas;
    const totalValue = txs.data.reduce((sum, { value }) => (value ? sum + value : sum), 0n);

    const requiredBalance = totalValue + totalGasLimit;
    let fees = totalGasLimit;

    if (shouldPayBridgingFee) fees += bridgingFee;

    return { requiredBalance, fees };
  };

  const txsEstimate = estimateTxs();

  const resetState = () => {
    mint.reset();
    approve.reset();
    permitUSDC.reset();
    transfer.reset();
  };

  const onSubmit = async (values: FormattedValues) => {
    definedAssert(txs.data, 'Prepared transactions');

    resetState();
    onTransactionStart(values);

    for (const { call } of txs.data) await call();
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

  return { onSubmit: mutateAsync, isPending, error, status, txsEstimate };
}

export { useHandleEthSubmit };
