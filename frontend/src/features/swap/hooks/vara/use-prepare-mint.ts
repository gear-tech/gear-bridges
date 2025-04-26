import { usePrepareProgramTransaction } from '@gear-js/react-hooks';

import { useWrappedVaraProgram } from '@/hooks';

function usePrepareMint() {
  const { data: program } = useWrappedVaraProgram();

  return {
    program,

    ...usePrepareProgramTransaction({
      program,
      serviceName: 'vftNativeExchange',
      functionName: 'mint',
    }),
  };
}

export { usePrepareMint };
