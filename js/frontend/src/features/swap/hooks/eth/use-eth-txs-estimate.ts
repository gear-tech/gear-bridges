import { useQuery } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { estimateFeesPerGas } from 'wagmi/actions';

import { useDebounce, useEthAccount } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import { DUMMY_ADDRESS } from '../../consts';
import { FormattedValues } from '../../types';
import { estimateBridging } from '../../utils';

import { usePrepareEthTxs } from './use-prepare-eth-txs';

type Params = {
  formValues: FormattedValues | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
};

const DUMMY_FORM_VALUES = {
  amount: 1n, // eth fails on 0
  accountAddress: DUMMY_ADDRESS.VARA_ALICE,
} as const;

function useEthTxsEstimate({ bridgingFee, shouldPayBridgingFee, formValues }: Params) {
  const ethAccount = useEthAccount();
  const config = useConfig();

  const ethTxs = usePrepareEthTxs({ bridgingFee, shouldPayBridgingFee });

  const estimateTxs = async () => {
    definedAssert(bridgingFee, 'Bridging fee');
    definedAssert(ethTxs.prepare, 'Prepared transactions');

    const txs = await ethTxs.prepare({
      ...(formValues ?? DUMMY_FORM_VALUES),
      accountOverride: ethAccount.address ? undefined : DUMMY_ADDRESS.ETH_DEAD,
      isEstimate: true,
    });

    const { maxFeePerGas } = await estimateFeesPerGas(config);
    const { totalGasLimit, totalValue } = estimateBridging(txs, maxFeePerGas);

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
