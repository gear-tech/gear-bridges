import { useSendProgramTransaction } from '@gear-js/react-hooks';

import { useWrappedVaraProgram } from '@/hooks';

function useBurnVaraTokens() {
  const { data: program } = useWrappedVaraProgram();

  return useSendProgramTransaction({
    program,
    serviceName: 'vftNativeExchange',
    functionName: 'burn',
  });
}

export { useBurnVaraTokens };
