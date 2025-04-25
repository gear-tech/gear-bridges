import { usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { useVFTProgram } from '@/hooks';

import { useBridgeContext } from '../../context';

function usePrepareApprove() {
  const { token } = useBridgeContext();
  const { data: program } = useVFTProgram(token.address);

  return usePrepareProgramTransaction({
    program,
    serviceName: 'vft',
    functionName: 'approve',
  });
}

export { usePrepareApprove };
