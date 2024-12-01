import { useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { VftManagerProgram } from '@/consts';
import { useVFTManagerAddress } from '@/features/swap';

function useFTAddresses() {
  const { data: vftManagerAddress } = useVFTManagerAddress();

  const { data: vftManagerProgram } = useProgram({
    library: VftManagerProgram,
    id: vftManagerAddress,
  });

  return useProgramQuery({
    program: vftManagerProgram,
    serviceName: 'vftManager',
    functionName: 'varaToEthAddresses',
    args: [],
  });
}

export { useFTAddresses };
