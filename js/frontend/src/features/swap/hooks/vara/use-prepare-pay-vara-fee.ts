import { usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { useBridgingPaymentProgram } from './use-bridging-payment-program';

function usePreparePayVaraFee() {
  const { data: program } = useBridgingPaymentProgram();

  return {
    program,

    ...usePrepareProgramTransaction({
      program,
      serviceName: 'bridgingPayment',
      functionName: 'payFees',
    }),
  };
}

export { usePreparePayVaraFee };
