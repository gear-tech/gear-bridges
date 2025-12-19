import { useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { useNetworkType } from '@/context/network-type';
import { isUndefined } from '@/utils';

import { BridgingPaymentProgram } from '../../consts';

import { useVFTManagerProgram } from './use-vft-manager-program';

function useVaraFee() {
  const { NETWORK_PRESET } = useNetworkType();
  const { data: vftManagerProgram } = useVFTManagerProgram();

  const { data: bridgingPaymentProgram } = useProgram({
    library: BridgingPaymentProgram,
    id: NETWORK_PRESET.BRIDGING_PAYMENT_CONTRACT_ADDRESS,
  });

  const vftManagerConfig = useProgramQuery({
    program: vftManagerProgram,
    serviceName: 'vftManager',
    functionName: 'getConfig',
    args: [],
  });

  const bridgingPaymentState = useProgramQuery({
    program: bridgingPaymentProgram,
    serviceName: 'bridgingPayment',
    functionName: 'getState',
    args: [],
  });

  const coerce = (value: string | number | bigint | undefined) => (isUndefined(value) ? undefined : BigInt(value));

  const vftManagerFee = coerce(vftManagerConfig.data?.fee_incoming);
  const bridgingFee = coerce(bridgingPaymentState.data?.fee);
  const priorityFee = coerce(bridgingPaymentState.data?.priority_fee);
  const isLoading = vftManagerConfig.isPending || bridgingPaymentState.isPending;

  return { bridgingFee, vftManagerFee, priorityFee, isLoading };
}

export { useVaraFee };
