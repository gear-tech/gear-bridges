import { useBalanceFormat, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { isUndefined } from '@/utils';

import { BridgingPaymentProgram, CONTRACT_ADDRESS } from '../../consts';

function useVaraFee() {
  const { getFormattedBalanceValue } = useBalanceFormat();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: CONTRACT_ADDRESS.BRIDGING_PAYMENT,
  });

  const { data: config, isPending } = useProgramQuery({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'getState',
    args: [],
  });

  const fee = {
    value: !isUndefined(config?.fee) ? BigInt(config.fee) : undefined,
    formattedValue: !isUndefined(config?.fee) ? getFormattedBalanceValue(config.fee.toString()).toFixed() : undefined,
  };

  const isLoading = isPending;

  return { fee, isLoading };
}

export { useVaraFee };
