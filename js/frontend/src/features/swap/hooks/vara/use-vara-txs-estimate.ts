import { useAccount, useApi } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { useNetworkType } from '@/context/network-type';
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
  vftManagerFee: bigint | undefined;
};

function useVaraTxsEstimate({ formValues, bridgingFee, shouldPayBridgingFee, vftManagerFee }: Params) {
  const { api } = useApi();
  const { account, isAccountReady } = useAccount();
  const { NETWORK_PRESET } = useNetworkType();

  const { token } = useBridgeContext();

  const prepareTxs = usePrepareVaraTxs({
    bridgingFee,
    shouldPayBridgingFee,
    vftManagerFee,
  });

  const estimateTxs = async () => {
    definedAssert(formValues, 'Form values');
    definedAssert(vftManagerFee, 'VFT Manager fee');

    definedAssert(api, 'API');
    definedAssert(prepareTxs, 'Prepared transactions');

    const txs = await prepareTxs(formValues);
    const { totalGasLimit, totalValue } = estimateBridging(txs, api.valuePerGas.toBigInt());

    const totalEstimatedFee = txs.reduce((sum, { estimatedFee }) => sum + estimatedFee, 0n);
    const requiredBalance = totalGasLimit + totalEstimatedFee + totalValue + api.existentialDeposit.toBigInt();
    const fees = totalGasLimit + totalEstimatedFee;

    return { requiredBalance, fees };
  };

  const debouncedAmount = useDebounce(formValues?.amount?.toString());
  const debouncedAccountAddress = useDebounce(formValues?.accountAddress);

  return useQuery({
    queryKey: [
      'vara-txs-estimate',
      api?.genesisHash.toHex(),
      NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS,
      NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
      vftManagerFee?.toString(),
      shouldPayBridgingFee ? bridgingFee?.toString() : undefined,
      debouncedAmount,
      debouncedAccountAddress,
      shouldPayBridgingFee,
      token?.address,
      account?.address,
    ],

    queryFn: estimateTxs,

    enabled:
      (!shouldPayBridgingFee || !isUndefined(bridgingFee)) &&
      !isUndefined(vftManagerFee) &&
      Boolean(api && formValues && token && prepareTxs) &&
      isAccountReady,
  });
}

export { useVaraTxsEstimate };
