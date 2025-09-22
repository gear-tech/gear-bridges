import { definedAssert, isUndefined } from '@/utils';

import { SUBMIT_STATUS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

import { useApprove } from './use-approve';
import { useGetEthAllowance } from './use-get-eth-ft-allowance';
import { useMint } from './use-mint';
import { usePermitUSDC } from './use-permit-usdc';
import { useTransfer } from './use-transfer';

const TRANSFER_GAS_LIMIT_FALLBACK = 21000n * 10n;

type Transaction = {
  call: () => Promise<unknown>;
  gasLimit: bigint;
  value?: bigint;
};

type Params = {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  ftBalance: bigint | undefined;
};

function usePrepareEthTxs({ bridgingFee, shouldPayBridgingFee, ftBalance }: Params) {
  const { token } = useBridgeContext();

  const getAllowance = useGetEthAllowance(token?.address);

  const mint = useMint();
  const approve = useApprove();
  const permitUSDC = usePermitUSDC();
  const transfer = useTransfer(bridgingFee, shouldPayBridgingFee);

  const prepare = async ({ amount, accountAddress, isEstimate }: FormattedValues & { isEstimate?: boolean }) => {
    definedAssert(bridgingFee, 'Bridging fee');
    definedAssert(ftBalance, 'FT balance');
    definedAssert(token, 'Token');

    const txs: Transaction[] = [];
    const shouldMint = token.isNative;

    const allowance = await getAllowance();
    const shouldApprove = amount > allowance;

    const isUSDC = token.symbol.toLowerCase().includes('usdc');

    if (shouldMint) {
      const value = amount;
      const gasLimit = await mint.getGasLimit({ value });

      txs.push({
        call: () => mint.mutateAsync({ value }),
        gasLimit,
        value,
      });
    }

    let permit: Awaited<ReturnType<typeof permitUSDC.mutateAsync>> | undefined;

    if (shouldApprove) {
      if (isUSDC) {
        if (!isEstimate) {
          permit = await permitUSDC.mutateAsync(amount);
        }
      } else {
        const call = () => approve.mutateAsync({ amount });
        const gasLimit = await approve.getGasLimit({ amount });

        txs.push({ call, gasLimit });
      }
    }

    // transfer estimate will fail if allowance or balance is less than amount.
    // we're checking balance at the form level, but not for WETH because it's always getting minted
    const canEstimateTransfer = !shouldApprove && ftBalance >= amount;

    txs.push({
      call: () => transfer.mutateAsync({ amount, accountAddress, permit }),

      gasLimit: canEstimateTransfer
        ? await transfer.getGasLimit({ amount, accountAddress })
        : TRANSFER_GAS_LIMIT_FALLBACK,

      value: shouldPayBridgingFee ? bridgingFee : undefined,
    });

    return txs;
  };

  const resetState = () => {
    mint.reset();
    approve.reset();
    permitUSDC.reset();
    transfer.reset();
  };

  const getStatus = () => {
    if (mint.isPending || mint.error) return SUBMIT_STATUS.MINT;
    if (approve.isPending || approve.error) return SUBMIT_STATUS.APPROVE;
    if (permitUSDC.isPending || permitUSDC.error) return SUBMIT_STATUS.PERMIT;
    if (transfer.isPending || transfer.error) return SUBMIT_STATUS.BRIDGE;

    return SUBMIT_STATUS.SUCCESS;
  };

  return {
    prepare: !isUndefined(bridgingFee) && !!token ? prepare : undefined,
    resetState,
    status: getStatus(),
  };
}

export { usePrepareEthTxs };
