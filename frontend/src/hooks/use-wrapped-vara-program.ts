import { useProgram } from '@gear-js/react-hooks';

import { WrappedVaraProgram } from '@/consts';
import { useTokens } from '@/context';

function useWrappedVaraProgram() {
  const { tokens } = useTokens();

  // TODO: active filter
  const wrappedVaraAddress = tokens?.find(
    ({ network, isActive, isNative }) => isActive && isNative && network === 'vara',
  )?.address;

  return useProgram({
    library: WrappedVaraProgram,
    id: wrappedVaraAddress,
  });
}

export { useWrappedVaraProgram };
