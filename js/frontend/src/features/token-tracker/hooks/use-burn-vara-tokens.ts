import { useSendProgramTransaction } from '@gear-js/react-hooks';

import { useWrappedVaraProgram } from '@/hooks';
import { useTokens } from '@/context/tokens';
import { useTokenAdmission } from '@/features/swap/hooks/use-token-admission';

function useBurnVaraTokens() {
  const { data: program } = useWrappedVaraProgram();
  const { nativeToken } = useTokens();
  const admit = useTokenAdmission(nativeToken.vara, 'unwrap');

  const { sendTransactionAsync, isPending } = useSendProgramTransaction({
    program,
    serviceName: 'vftNativeExchange',
    functionName: 'burn',
  });

  const mutateAsync = async (value: bigint) => {
    await admit(nativeToken.vara);
    return sendTransactionAsync({ args: [value] });
  };

  return { isPending, mutateAsync };
}

export { useBurnVaraTokens };
