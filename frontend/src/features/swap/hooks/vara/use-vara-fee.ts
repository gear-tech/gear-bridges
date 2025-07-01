import { useBalanceFormat, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { isUndefined } from '@/utils';

import { BridgingPaymentProgram, CONTRACT_ADDRESS, VftManagerProgram } from '../../consts';

function useVaraFee() {
  const { getFormattedBalanceValue } = useBalanceFormat();

  const { data: vftManagerProgram } = useProgram({
    library: VftManagerProgram,
    id: CONTRACT_ADDRESS.VFT_MANAGER,
  });

  const { data: bridgingPaymentProgram } = useProgram({
    library: BridgingPaymentProgram,
    id: CONTRACT_ADDRESS.BRIDGING_PAYMENT,
  });

  const vftManagerConfig = useProgramQuery({
    program: vftManagerProgram,
    serviceName: 'vftManager',
    functionName: 'getConfig',
    args: [],
  });

  const bridgingPaymentState = useProgramQuery({
    program: bridgingPaymentProgram,
    serviceName: 'bridgingPayment',
    functionName: 'getState',
    args: [],
  });

  const vftManagerFee = {
    value: !isUndefined(vftManagerConfig.data?.fee_incoming) ? BigInt(vftManagerConfig.data.fee_incoming) : undefined,

    formattedValue: !isUndefined(vftManagerConfig.data?.fee_incoming)
      ? getFormattedBalanceValue(vftManagerConfig.data.fee_incoming.toString()).toFixed()
      : undefined,
  };

  const bridgingFee = {
    value: !isUndefined(bridgingPaymentState.data?.fee) ? BigInt(bridgingPaymentState.data.fee) : undefined,

    formattedValue: !isUndefined(bridgingPaymentState.data?.fee)
      ? getFormattedBalanceValue(bridgingPaymentState.data.fee.toString()).toFixed()
      : undefined,
  };

  const isLoading = vftManagerConfig.isPending || bridgingPaymentState.isPending;

  return { bridgingFee, vftManagerFee, isLoading };
}

export { useVaraFee };
