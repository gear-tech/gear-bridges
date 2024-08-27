import { useBalanceFormat, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { isUndefined } from '@/utils';

import { BridgingPaymentProgram } from '../../consts';

const BRIDGING_PAYMENT_ADDRESS = '0xb9c7edd377b31834bfd539497eafa49e77752cf79cf5521f5de8fef041e45d1c';

function useVaraConfig(enabled: boolean) {
  const { getFormattedBalance } = useBalanceFormat();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_ADDRESS,
    query: { enabled },
  });

  const { data: config, isPending } = useProgramQuery({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'getConfig',
    args: [],
  });

  const fee = {
    value: !isUndefined(config?.fee) ? BigInt(config.fee) : undefined,
    formattedValue: !isUndefined(config?.fee) ? getFormattedBalance(config.fee).value : undefined,
  };

  const isLoading = isPending;

  return { fee, isLoading };
}

export { useVaraConfig };
