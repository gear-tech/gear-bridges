import { usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { useBridgingPaymentProgram } from './use-bridging-payment-program';

function usePreparePayPriorityFee() {
  const { data: program } = useBridgingPaymentProgram();

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
