import { usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { useVFTManagerProgram } from './use-vft-manager-program';

function usePrepareRequestBridging() {
  const { data: program } = useVFTManagerProgram();

  return {
    program,

    ...usePrepareProgramTransaction({
      program,
      serviceName: 'vftManager',
      functionName: 'requestBridging',
    }),
  };
}

export { usePrepareRequestBridging };
