import { useProgram, usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { BridgingPaymentProgram, CONTRACT_ADDRESS } from '../../consts';

function usePreparePayVaraFee() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: CONTRACT_ADDRESS.BRIDGING_PAYMENT,
  });

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
