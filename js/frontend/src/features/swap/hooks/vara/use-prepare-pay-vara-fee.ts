import { useProgram, usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { useNetworkType } from '@/context/network-type';

import { BridgingPaymentProgram } from '../../consts';

function usePreparePayVaraFee() {
  const { NETWORK_PRESET } = useNetworkType();

  const { data: program } = useProgram({
    library: BridgingPaymentProgram,
    id: NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
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
