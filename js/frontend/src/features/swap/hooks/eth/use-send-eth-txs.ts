import { useMutation } from '@tanstack/react-query';

import { definedAssert } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useMint } from './use-mint';
import { usePermitUSDC } from './use-permit-usdc';
import { usePrepareEthTxs } from './use-prepare-eth-txs';
import { useTransfer } from './use-transfer';

type Params = {
  allowance: bigint | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  onTransactionStart: (values: FormattedValues) => void;
};

function useSendEthTxs({ allowance, bridgingFee, shouldPayBridgingFee, onTransactionStart }: Params) {
  const mint = useMint();
  const approve = useApprove();
  const permitUSDC = usePermitUSDC();
  const transfer = useTransfer(bridgingFee, shouldPayBridgingFee);

  const prepareTxs = usePrepareEthTxs({ allowance, bridgingFee, shouldPayBridgingFee });

  const resetState = () => {
    mint.reset();
    approve.reset();
    permitUSDC.reset();
    transfer.reset();
  };

  const sendTxs = async (values: FormattedValues) => {
    definedAssert(prepareTxs, 'Prepared transactions');

    const txs = await prepareTxs(values);

    resetState();
    onTransactionStart(values);

    for (const { call } of txs) await call();
  };

  const getStatus = () => {
    if (mint.isPending || mint.error) return SUBMIT_STATUS.MINT;
    if (approve.isPending || approve.error) return SUBMIT_STATUS.APPROVE;
    if (permitUSDC.isPending || permitUSDC.error) return SUBMIT_STATUS.PERMIT;
    if (transfer.isPending || transfer.error) return SUBMIT_STATUS.BRIDGE;

    return SUBMIT_STATUS.SUCCESS;
  };

  return { ...useMutation({ mutationFn: sendTxs }), status: getStatus() };
}

export { useSendEthTxs };
