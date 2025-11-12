import { HexString } from '@gear-js/api';
import { useAccount } from '@gear-js/react-hooks';

import { useNetworkType } from '@/context/network-type';
import { useVFTProgram } from '@/hooks';
import { definedAssert } from '@/utils';

function useGetVaraFTAllowance(address: HexString | undefined) {
  const { NETWORK_PRESET } = useNetworkType();
  const { account } = useAccount();
  const { data: program } = useVFTProgram(address);

  return () => {
    definedAssert(program, 'VFT program');
    definedAssert(address, 'FT address');
    definedAssert(account?.decodedAddress, 'Account address');

    return program.vft
      .allowance(account.decodedAddress, NETWORK_PRESET.VFT_MANAGER_CONTRACT_ADDRESS)
      .withAddress(account.decodedAddress)
      .call();
  };
}

export { useGetVaraFTAllowance };
