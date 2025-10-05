import { HexString } from '@gear-js/api';
import { useAccount } from '@gear-js/react-hooks';

import { useVFTProgram } from '@/hooks';
import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';

function useGetVaraFTAllowance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useVFTProgram(address);

  return () => {
    definedAssert(program, 'VFT program');
    definedAssert(address, 'FT address');
    definedAssert(account?.decodedAddress, 'Account address');

    return program.vft
      .allowance(account.decodedAddress, CONTRACT_ADDRESS.VFT_MANAGER)
      .withAddress(account.decodedAddress)
      .call();
  };
}

export { useGetVaraFTAllowance };
