import { useQuery } from '@tanstack/react-query';
import { useConfig } from 'wagmi';
import { estimateFeesPerGas } from 'wagmi/actions';

import { useDebounce, useEthAccount } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';
import { estimateBridging } from '../../utils';

import { usePrepareEthTxs } from './use-prepare-eth-txs';

type Params = {
  formValues: FormattedValues | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  ftBalance: bigint | undefined;
};

function useEthTxsEstimate({ bridgingFee, shouldPayBridgingFee, formValues, ftBalance }: Params) {
  const ethAccount = useEthAccount();
  const config = useConfig();

  const { token } = useBridgeContext();

  const ethTxs = usePrepareEthTxs({ bridgingFee, shouldPayBridgingFee, ftBalance });

  const estimateTxs = async () => {
    definedAssert(formValues, 'Form values');
    definedAssert(token, 'Fungible Token');
    definedAssert(ethTxs.prepare, 'Prepared transactions');

    const txs = await ethTxs.prepare({ ...formValues, isEstimate: true });

    const { maxFeePerGas } = await estimateFeesPerGas(config);
    const { totalGasLimit, totalValue } = estimateBridging(txs, maxFeePerGas);

    const requiredBalance = totalValue + totalGasLimit;
    const fees = totalGasLimit;

    return { requiredBalance, fees };
  };

  const debouncedAmount = useDebounce(formValues?.amount?.toString());
  const debouncedAccountAddress = useDebounce(formValues?.accountAddress);

  return useQuery({
    queryKey: [
      'eth-txs-estimate',
      debouncedAmount,
      debouncedAccountAddress,
      shouldPayBridgingFee,
      token?.address,
      ethAccount.address,
    ],

    queryFn: estimateTxs,

    enabled: Boolean(!isUndefined(bridgingFee) && formValues && token && ethAccount.address),
  });
}

export { useEthTxsEstimate };
