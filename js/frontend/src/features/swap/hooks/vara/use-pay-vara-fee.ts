import { useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';

import { useNetworkType } from '@/context/network-type';

import { BridgingPaymentProgram } from '../../consts';

function usePayVaraFee() {
  const { NETWORK_PRESET } = useNetworkType();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  return useSendProgramTransaction({
    program,
    serviceName: 'bridgingPayment',
    functionName: 'payFees',
  });
}

export { usePayVaraFee };
