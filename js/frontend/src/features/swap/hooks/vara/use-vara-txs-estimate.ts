import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { useDebounce } from '@/hooks';
import { definedAssert, isUndefined } from '@/utils';

import { DUMMY_ADDRESS } from '../../consts';
import { useBridgeContext } from '../../context';
import { FormattedValues } from '../../types';
import { estimateBridging } from '../../utils';

import { usePrepareVaraTxs } from './use-prepare-vara-txs';

type Params = {
  formValues: FormattedValues | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  vftManagerFee: bigint | undefined;
};

const DUMMY_FORM_VALUES = {
  amount: 0n,
  accountAddress: DUMMY_ADDRESS.ETH_DEAD,
} as const;

function useVaraTxsEstimate({ formValues, bridgingFee, shouldPayBridgingFee, vftManagerFee }: Params) {
  const { api } = useApi();
  const { account, isAccountReady } = useAccount();

  const { token } = useBridgeContext();

  const prepareTxs = usePrepareVaraTxs({ bridgingFee, shouldPayBridgingFee, vftManagerFee });

  const estimateTxs = async () => {
    definedAssert(vftManagerFee, 'VFT Manager fee');
    definedAssert(bridgingFee, 'Bridging fee value');
    definedAssert(api, 'API');
    definedAssert(prepareTxs, 'Prepared transactions');

    const txs = await prepareTxs({
      ...(formValues ?? DUMMY_FORM_VALUES),
      accountOverride: account ? undefined : DUMMY_ADDRESS.VARA_ALICE,
    });

    const { totalGasLimit, totalValue } = estimateBridging(txs, api.valuePerGas.toBigInt());

    const totalEstimatedFee = txs.reduce((sum, { estimatedFee }) => sum + estimatedFee, 0n);
    const requiredBalance = totalGasLimit + totalEstimatedFee + totalValue + api.existentialDeposit.toBigInt();

    let fees = totalGasLimit + totalEstimatedFee + vftManagerFee;
    if (shouldPayBridgingFee) fees += bridgingFee;

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
      !isUndefined(bridgingFee) && !isUndefined(vftManagerFee) && Boolean(api && token && prepareTxs) && isAccountReady,
  });
}

export { useVaraTxsEstimate };
