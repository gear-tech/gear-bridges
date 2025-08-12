import { useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';

import { BridgingPaymentProgram, CONTRACT_ADDRESS } from '../../consts';

function usePayVaraFee() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: CONTRACT_ADDRESS.BRIDGING_PAYMENT,
  });

  return useSendProgramTransaction({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'payFees',
  });
}

export { usePayVaraFee };
