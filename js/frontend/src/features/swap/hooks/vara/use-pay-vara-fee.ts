import { useSendProgramTransaction } from '@gear-js/react-hooks';

import { useBridgingPaymentProgram } from './use-bridging-payment-program';

function usePayVaraFee() {
  const { data: program } = useBridgingPaymentProgram();

  return useSendProgramTransaction({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'payFees',
  });
}

export { usePayVaraFee };
