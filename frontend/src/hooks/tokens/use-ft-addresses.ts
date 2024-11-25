import { HexString } from '@gear-js/api';
import { useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS, VftManagerProgram } from '@/consts';

function useFTAddresses() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const { data: vftManagerAddress } = useProgramQuery({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'vftManagerAddress',
    args: [],
  });

  const { data: vftManagerProgram } = useProgram({
    library: VftManagerProgram,
    id: vftManagerAddress?.toString() as HexString,
  });

  return useProgramQuery({
    program: vftManagerProgram,
    serviceName: 'vftManager',
    functionName: 'varaToEthAddresses',
    args: [],
  });
}

export { useFTAddresses };
