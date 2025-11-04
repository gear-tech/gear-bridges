import { useProgram } from '@gear-js/react-hooks';

import { VftManagerProgram, CONTRACT_ADDRESS } from '../../consts';

function useVFTManagerProgram() {
  return useProgram({
    library: VftManagerProgram,
    id: CONTRACT_ADDRESS.VFT_MANAGER,
  });
}

export { useVFTManagerProgram };
