import { useBalanceFormat, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '@/consts';
import { isUndefined } from '@/utils';

import { QUERY_NAME, SERVICE_NAME } from '../../consts';

function useVaraFee() {
  const { getFormattedBalance } = useBalanceFormat();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const { data: config, isPending } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME.BRIDGING_PAYMENT,
    functionName: QUERY_NAME.GET_CONFIG,
    args: [],
  });

  const fee = {
    value: !isUndefined(config?.fee) ? BigInt(config.fee) : undefined,
    formattedValue: !isUndefined(config?.fee) ? getFormattedBalance(config.fee).value : undefined,
  };

  const isLoading = isPending;

  return { fee, isLoading };
}

export { useVaraFee };
