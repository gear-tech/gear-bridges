import { useProgram, usePrepareProgramTransaction, useSendProgramTransaction } from '@gear-js/react-hooks';

import { WrappedVaraProgram, WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';

function useMint() {
  const { data: program } = useProgram({
    library: WrappedVaraProgram,
    id: WRAPPED_VARA_CONTRACT_ADDRESS,
  });

  const params = { program, serviceName: 'vftNativeExchange' as const, functionName: 'mint' as const };
  const { prepareTransactionAsync } = usePrepareProgramTransaction(params);
  const send = useSendProgramTransaction(params);

  return { ...send, prepareTransactionAsync };
}

export { useMint };
