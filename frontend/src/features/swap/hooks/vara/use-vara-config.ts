import { useBalanceFormat, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS } from '@/consts';
import { isUndefined } from '@/utils';

import { QUERY_NAME, SERVICE_NAME } from '../../consts';

function useVaraConfig(enabled: boolean) {
  const { getFormattedBalance } = useBalanceFormat();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
    query: { enabled },
  });

  const { data: config, isPending } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME.BRIDGING_PAYMENT,
    functionName: QUERY_NAME.GET_CONFIG,
    args: [],
    query: { enabled },
  });

  if (!enabled) return { fee: { value: BigInt(0), formattedValue: '0' }, isLoading: false };

  const fee = {
    value: !isUndefined(config?.fee) ? BigInt(config.fee) : undefined,
    formattedValue: !isUndefined(config?.fee) ? getFormattedBalance(config.fee).value : undefined,
  };

  const isLoading = isPending;

  return { fee, isLoading };
}

export { useVaraConfig };
