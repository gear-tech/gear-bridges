import { useProgram } from '@gear-js/react-hooks';

import { useNetworkType } from '@/context/network-type';

import { VftManagerProgram } from '../../consts';

function useVFTManagerProgram() {
  const { NETWORK_PRESET } = useNetworkType();

  return useProgram({
    library: VftManagerProgram,
    id: NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS,
  });
}

export { useVFTManagerProgram };
