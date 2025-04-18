import { HexString } from '@gear-js/api';
import { useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { VFT_MANAGER_CONTRACT_ADDRESS, VftManagerProgram } from '@/consts';

function useFTAddresses() {
  const { data: vftManagerProgram } = useProgram({
    library: VftManagerProgram,
    id: VFT_MANAGER_CONTRACT_ADDRESS,
  });

  return useProgramQuery({
    program: vftManagerProgram,
    serviceName: 'vftManager',
    functionName: 'varaToEthAddresses',
    args: [],
    query: {
      select: (data) => data.map((pair) => [pair[0].toString(), pair[1].toString()] as [HexString, HexString]),
    },
  });
}

export { useFTAddresses };
