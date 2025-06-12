import { useProgram } from '@gear-js/react-hooks';

import { WrappedVaraProgram } from '@/consts';

import { useTokens } from './use-tokens';

function useWrappedVaraProgram() {
  const { wrappedVaraAddress } = useTokens();

  return useProgram({
    library: WrappedVaraProgram,
    id: wrappedVaraAddress,
  });
}

export { useWrappedVaraProgram };
