import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { useDebounce } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';
import { estimateBridging } from '../../utils';

import { usePrepareVaraTxs } from './use-prepare-vara-txs';

type Params = {
  formValues: FormattedValues | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  priorityFee: bigint | undefined;
  shouldPayPriorityFee: boolean;
  vftManagerFee: bigint | undefined;
};

function useVaraTxsEstimate({
  formValues,
  bridgingFee,
  shouldPayBridgingFee,
  priorityFee,
  shouldPayPriorityFee,
  vftManagerFee,
}: Params) {
  const { api } = useApi();
  const { account, isAccountReady } = useAccount();

  const { token } = useBridgeContext();

  const prepareTxs = usePrepareVaraTxs({
    bridgingFee,
    shouldPayBridgingFee,
    priorityFee,
    shouldPayPriorityFee,
    vftManagerFee,
  });

  const estimateTxs = async () => {
    definedAssert(formValues, 'Form values');
    definedAssert(vftManagerFee, 'VFT Manager fee');
    definedAssert(bridgingFee, 'Bridging fee value');
    definedAssert(priorityFee, 'Priority fee value');
    definedAssert(api, 'API');
    definedAssert(prepareTxs, 'Prepared transactions');

    const txs = await prepareTxs(formValues);
    const { totalGasLimit, totalValue } = estimateBridging(txs, api.valuePerGas.toBigInt());

    const totalEstimatedFee = txs.reduce((sum, { estimatedFee }) => sum + estimatedFee, 0n);
    const requiredBalance = totalGasLimit + totalEstimatedFee + totalValue + api.existentialDeposit.toBigInt();

    let fees = totalGasLimit + totalEstimatedFee + vftManagerFee;

    if (shouldPayBridgingFee) fees += bridgingFee;
    if (shouldPayPriorityFee) fees += priorityFee;

    return { requiredBalance, fees };
  };

  const debouncedAmount = useDebounce(formValues?.amount?.toString());
  const debouncedAccountAddress = useDebounce(formValues?.accountAddress);

  return useQuery({
    queryKey: [
      'vara-txs-estimate',
      debouncedAmount,
      debouncedAccountAddress,
      shouldPayBridgingFee,
      token?.address,
      account?.address,
    ],

    queryFn: estimateTxs,

    enabled:
      !isUndefined(bridgingFee) &&
      !isUndefined(vftManagerFee) &&
      Boolean(api && formValues && token && prepareTxs) &&
      isAccountReady,
  });
}

export { useVaraTxsEstimate };
