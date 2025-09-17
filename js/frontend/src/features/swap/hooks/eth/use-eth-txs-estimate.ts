import { useQuery } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { estimateFeesPerGas } from 'wagmi/actions';

import { useDebounce, useEthAccount } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import { DUMMY_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';
import { estimateBridging } from '../../utils';

import { usePrepareEthTxs } from './use-prepare-eth-txs';

type Params = {
  formValues: FormattedValues | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  accountBalance: bigint | undefined;
};

const DUMMY_FORM_VALUES = {
  amount: 1n, // eth fails on 0
  accountAddress: DUMMY_ADDRESS.VARA_ALICE,
} as const;

function useEthTxsEstimate({ bridgingFee, shouldPayBridgingFee, formValues, accountBalance = 0n }: Params) {
  const ethAccount = useEthAccount();
  const config = useConfig();

  const { token } = useBridgeContext();

  const ethTxs = usePrepareEthTxs({ bridgingFee, shouldPayBridgingFee });

  const estimateTxs = async () => {
    definedAssert(bridgingFee, 'Bridging fee');
    definedAssert(token, 'Fungible Token');
    definedAssert(ethTxs.prepare, 'Prepared transactions');

    const safeValues = formValues ?? DUMMY_FORM_VALUES;

    const feeValue = shouldPayBridgingFee ? bridgingFee : 0n;
    const amountValue = token.isNative ? safeValues.amount : 0n;

    // if balance is insufficient - estimate gas will fail, so we have to use dummy account with existing balance.
    // it should be used dummy values as well, in case if user amount input is too high
    const isDummyAccount = amountValue + feeValue >= accountBalance;

    const txs = await ethTxs.prepare({
      ...(isDummyAccount ? DUMMY_FORM_VALUES : safeValues),
      accountOverride: isDummyAccount ? DUMMY_ADDRESS.ETH_DEAD : undefined,
      isEstimate: true,
    });

    const { maxFeePerGas } = await estimateFeesPerGas(config);
    const { totalGasLimit, totalValue } = estimateBridging(txs, maxFeePerGas);

    // it's feasible to calculate required balance using prepared txs as a single source of truth,
    // but whenever dummy amount is used (and since it has to be used whenever dummy account is used)
    // - it will be incorrect most of the time.
    // maybe we will figure out a better way to estimate gas for arbitrary amounts later,
    // maybe it's worth to consider to just using constant (heuristic) gas limit fallbacks on failed estimates,
    // but for now leaving it as is, even though it's useless
    const requiredBalance = totalValue + totalGasLimit;

    let fees = totalGasLimit;
    if (shouldPayBridgingFee) fees += bridgingFee;

    return { requiredBalance, fees };
  };

  const debouncedAmount = useDebounce(formValues?.amount?.toString());
  const debouncedAccountAddress = useDebounce(formValues?.accountAddress);

  return useQuery({
    queryKey: ['eth-txs-estimate', debouncedAmount, debouncedAccountAddress, shouldPayBridgingFee, ethAccount.address],

    queryFn: estimateTxs,

    // it's probably worth to check isConnecting too, but there is a bug:
    // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
    enabled: Boolean(!isUndefined(bridgingFee) && !ethAccount.isReconnecting),
  });
}

export { useEthTxsEstimate };
