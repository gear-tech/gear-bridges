import { usePrepareProgramTransaction, useProgram } from '@gear-js/react-hooks';

import { BridgingPaymentProgram, CONTRACT_ADDRESS } from '../../consts';

function usePreparePayPriorityFee() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: CONTRACT_ADDRESS.BRIDGING_PAYMENT,
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
