import { HexString } from '@gear-js/api';
import { useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { BRIDGING_PAYMENT_CONTRACT_ADDRESS, BridgingPaymentProgram } from '../../consts';

function useVFTManagerAddress() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  return useProgramQuery({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'vftManagerAddress',
    args: [],
    query: { select: (value) => value.toString() as HexString },
  });
}

export { useVFTManagerAddress };
