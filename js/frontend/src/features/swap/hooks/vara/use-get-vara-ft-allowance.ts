import { HexString } from '@gear-js/api';
import { useAccount } from '@gear-js/react-hooks';

import { useVFTProgram } from '@/hooks';
import { definedAssert } from '@/utils';

import { CONTRACT_ADDRESS } from '../../consts';

function useGetVaraFTAllowance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useVFTProgram(address);

  return (accountOverride: HexString | undefined) => {
    const accountAddress = account?.decodedAddress || accountOverride;

    definedAssert(program, 'VFT program');
    definedAssert(address, 'FT address');
    definedAssert(accountAddress, 'Allowance account address');

    return program.vft.allowance(accountAddress, CONTRACT_ADDRESS.VFT_MANAGER);
  };
}

export { useGetVaraFTAllowance };
