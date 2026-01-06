import { useProgram } from '@gear-js/react-hooks';

import { useNetworkType } from '@/context/network-type';

import { BridgingPaymentProgram } from '../../consts';

function useBridgingPaymentProgram() {
  const { NETWORK_PRESET } = useNetworkType();

  return useProgram({
    library: BridgingPaymentProgram,
    id: NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });
}

export { useBridgingPaymentProgram };
