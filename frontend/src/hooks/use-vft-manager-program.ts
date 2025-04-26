import { useProgram } from '@gear-js/react-hooks';

import { VftManagerProgram, VFT_MANAGER_CONTRACT_ADDRESS } from '@/consts';

function useVFTManagerProgram() {
  return useProgram({
    library: VftManagerProgram,
    id: VFT_MANAGER_CONTRACT_ADDRESS,
  });
}

export { useVFTManagerProgram };
