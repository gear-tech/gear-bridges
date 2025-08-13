import { HexString } from '@gear-js/api';
import { useProgram } from '@gear-js/react-hooks';

import { VftProgram } from '@/consts';

function useVFTProgram(address: HexString | undefined) {
  return useProgram({
    library: VftProgram,
    id: address,
  });
}

export { useVFTProgram };
