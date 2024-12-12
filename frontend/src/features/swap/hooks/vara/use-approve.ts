import { HexString } from '@gear-js/api';
import { useProgram, usePrepareProgramTransaction, useSendProgramTransaction } from '@gear-js/react-hooks';

import { VftProgram } from '@/consts';

import { SERVICE_NAME } from '../../consts';
import { FUNCTION_NAME } from '../../consts/vara';

function useApprove(ftAddress: HexString | undefined) {
  const { data: program } = useProgram({
    library: VftProgram,
    id: ftAddress,
  });

  const params = { program, serviceName: SERVICE_NAME.VFT, functionName: FUNCTION_NAME.APPROVE };
  const { prepareTransactionAsync } = usePrepareProgramTransaction(params);
  const send = useSendProgramTransaction(params);

  return { ...send, prepareTransactionAsync };
}

export { useApprove };
