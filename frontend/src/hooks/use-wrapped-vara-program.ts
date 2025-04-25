import { useProgram } from '@gear-js/react-hooks';

import { WRAPPED_VARA_CONTRACT_ADDRESS, WrappedVaraProgram } from '@/consts';

function useWrappedVaraProgram() {
  return useProgram({
    library: WrappedVaraProgram,
    id: WRAPPED_VARA_CONTRACT_ADDRESS,
  });
}

export { useWrappedVaraProgram };
