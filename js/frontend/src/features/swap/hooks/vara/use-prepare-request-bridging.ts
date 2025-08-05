import { usePrepareProgramTransaction, useProgram } from '@gear-js/react-hooks';

import { CONTRACT_ADDRESS, VftManagerProgram } from '../../consts';

function usePrepareRequestBridging() {
  const { data: program } = useProgram({
    library: VftManagerProgram,
    id: CONTRACT_ADDRESS.VFT_MANAGER,
  });

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
