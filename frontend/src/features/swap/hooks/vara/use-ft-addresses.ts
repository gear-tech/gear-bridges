import { HexString } from '@gear-js/api';
import { useProgram, useProgramQuery } from '@gear-js/react-hooks';

import {
  BridgingPaymentProgram,
  BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  VftGatewayProgram,
  SERVICE_NAME,
  QUERY_NAME,
} from '../../consts';

function useFTAddresses() {
  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const { data: vftGatewayAddress } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME.BRIDGING_PAYMENT,
    functionName: QUERY_NAME.VFT_GATEWAY_ADDRESS,
    args: [],
  });

  const { data: vftGatewayProgram } = useProgram({
    library: VftGatewayProgram,
    id: vftGatewayAddress?.toString() as HexString,
  });

  return useProgramQuery({
    program: vftGatewayProgram,
    serviceName: SERVICE_NAME.VFT_GATEWAY,
    functionName: QUERY_NAME.FT_ADDRESSES,
    args: [],
  });
}

export { useFTAddresses };
