import { useProgram, useSendProgramTransaction } from '@gear-js/react-hooks';

import { WrappedVaraProgram, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';

function useBurnVaraTokens() {
  const { data: program } = useProgram({
    library: WrappedVaraProgram,
    id: WRAPPED_VARA_CONTRACT_ADDRESS,
  });

  return useSendProgramTransaction({
    program,
    serviceName: 'tokenizer',
    functionName: 'burn',
  });
}

export { useBurnVaraTokens };
