import { useProgram } from '@gear-js/react-hooks';

import { WrappedVaraProgram } from '@/consts';
import { useTokens } from '@/context';

function useWrappedVaraProgram() {
  const { nativeToken } = useTokens();

  return useProgram({
    library: WrappedVaraProgram,
    id: nativeToken.vara?.address,
  });
}

export { useWrappedVaraProgram };
