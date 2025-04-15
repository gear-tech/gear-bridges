import { useProgram, usePrepareProgramTransaction, useSendProgramTransaction, useApi } from '@gear-js/react-hooks';

import { VFT_MANAGER_CONTRACT_ADDRESS, VftManagerProgram } from '@/consts';

function useTransferGasLimit() {
  const { api } = useApi();

  return api?.blockGasLimit.toBigInt();
}

function useTransfer() {
  const { data: program } = useProgram({
    library: VftManagerProgram,
    id: VFT_MANAGER_CONTRACT_ADDRESS,
  });

  const params = { program, serviceName: 'vftManager' as const, functionName: 'requestBridging' as const };
  const { prepareTransactionAsync } = usePrepareProgramTransaction(params);
  const send = useSendProgramTransaction(params);

  return { ...send, prepareTransactionAsync };
}

export { useTransferGasLimit, useTransfer };
