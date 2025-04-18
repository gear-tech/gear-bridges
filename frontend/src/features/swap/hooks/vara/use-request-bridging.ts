import { useProgram, usePrepareProgramTransaction, useSendProgramTransaction } from '@gear-js/react-hooks';

import { VFT_MANAGER_CONTRACT_ADDRESS, VftManagerProgram } from '@/consts';

function useRequestBridging() {
  const { data: program } = useProgram({
    library: VftManagerProgram,
    id: VFT_MANAGER_CONTRACT_ADDRESS,
  });

  const params = { program, serviceName: 'vftManager' as const, functionName: 'requestBridging' as const };
  const { prepareTransactionAsync } = usePrepareProgramTransaction(params);
  const send = useSendProgramTransaction(params);

  return { ...send, prepareTransactionAsync };
}

export { useRequestBridging };
