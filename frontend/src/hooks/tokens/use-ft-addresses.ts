import { HexString } from '@gear-js/api';
import { useProgramQuery } from '@gear-js/react-hooks';

import { useVFTManagerProgram } from '../use-vft-manager-program';

function useFTAddresses() {
  const { data: vftManagerProgram } = useVFTManagerProgram();

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
