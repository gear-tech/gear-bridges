import { usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { useVFTManagerProgram } from '@/hooks';

function usePrepareRequestBridging() {
  const { data: program } = useVFTManagerProgram();

  return usePrepareProgramTransaction({
    program,
    serviceName: 'vftManager',
    functionName: 'requestBridging',
  });
}

export { usePrepareRequestBridging };
