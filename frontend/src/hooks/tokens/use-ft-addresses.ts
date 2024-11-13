import { HexString } from '@gear-js/api';
import { useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { BridgingPaymentProgram, BRIDGING_PAYMENT_CONTRACT_ADDRESS, VftGatewayProgram } from '@/consts';

function useFTAddresses() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const { data: vftGatewayAddress } = useProgramQuery({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'vftGatewayAddress',
    args: [],
  });

  const { data: vftGatewayProgram } = useProgram({
    library: VftGatewayProgram,
    id: vftGatewayAddress?.toString() as HexString,
  });

  return useProgramQuery({
    program: vftGatewayProgram,
    serviceName: 'vftGateway',
    functionName: 'varaToEthAddresses',
    args: [],
  });
}

export { useFTAddresses };
