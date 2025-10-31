import { usePrepareProgramTransaction, useProgram } from '@gear-js/react-hooks';

import { useNetworkType } from '@/context';

import { BridgingPaymentProgram } from '../../consts';

function usePreparePayPriorityFee() {
  const { NETWORK_PRESET } = useNetworkType();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  return {
    program,

    ...usePrepareProgramTransaction({
      program,
      serviceName: 'bridgingPayment',
      functionName: 'payPriorityFees',
    }),
  };
}

export { usePreparePayPriorityFee };
