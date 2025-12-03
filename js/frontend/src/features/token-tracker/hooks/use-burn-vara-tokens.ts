import { useSendProgramTransaction } from '@gear-js/react-hooks';

import { useWrappedVaraProgram } from '@/hooks';

function useBurnVaraTokens() {
  const { data: program } = useWrappedVaraProgram();

  const { sendTransactionAsync, isPending } = useSendProgramTransaction({
    program,
    serviceName: 'vftNativeExchange',
    functionName: 'burn',
  });

  const mutateAsync = async (value: bigint) => sendTransactionAsync({ args: [value] });

  return { isPending, mutateAsync };
}

export { useBurnVaraTokens };
