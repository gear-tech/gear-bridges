import { HexString } from '@gear-js/api';

import { isUndefined } from '@/utils';

import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';

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

type Params = {
  allowance: bigint | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
};

function usePrepareEthTxs({ allowance, bridgingFee, shouldPayBridgingFee }: Params) {
  const { token } = useBridgeContext();

  const mint = useMint();
  const approve = useApprove();
  const permitUSDC = usePermitUSDC();
  const transfer = useTransfer(bridgingFee, shouldPayBridgingFee);

  if (isUndefined(allowance) || isUndefined(bridgingFee) || !token) return;

  return async ({
    amount,
    accountAddress,
    accountOverride,
    isEstimate,
  }: FormattedValues & { accountOverride?: HexString; isEstimate?: boolean }) => {
    const txs: Transaction[] = [];
    const shouldMint = token.isNative;
    const shouldApprove = amount > allowance;
    const isUSDC = token.symbol.toLowerCase().includes('usdc');

    if (shouldMint) {
      const value = amount;
      const gasLimit = await mint.getGasLimit({ value, accountOverride });

      txs.push({
        call: () => mint.mutateAsync({ value }),
        gasLimit,
        value,
      });
    }

    let permit: Awaited<ReturnType<typeof permitUSDC.mutateAsync>> | undefined;

    if (shouldApprove) {
      if (isUSDC && !isEstimate) {
        permit = await permitUSDC.mutateAsync(amount);
      } else {
        const call = () => approve.mutateAsync({ amount });
        const gasLimit = await approve.getGasLimit({ amount, accountOverride });

        txs.push({ call, gasLimit });
      }
    }

    // if approve is not made, transfer gas estimate will fail.
    // it can be avoided by using stateOverride,
    // but it requires the knowledge of the storage slot or state diff of the allowance for each token,
    // which is not feasible to do programmatically (at least I didn't managed to find a convenient way to do so).
    txs.push({
      call: () => transfer.mutateAsync({ amount, accountAddress, permit }),

      gasLimit: shouldApprove
        ? TRANSFER_GAS_LIMIT_FALLBACK
        : await transfer.getGasLimit({ amount, accountAddress, accountOverride }),

      value: shouldPayBridgingFee ? bridgingFee : undefined,
    });

    return txs;
  };
}

export { usePrepareEthTxs };
