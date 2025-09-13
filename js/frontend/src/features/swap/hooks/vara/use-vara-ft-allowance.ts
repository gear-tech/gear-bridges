import { HexString } from '@gear-js/api';
import { useAccount, useProgramQuery } from '@gear-js/react-hooks';

import { useVFTProgram } from '@/hooks';

import { CONTRACT_ADDRESS } from '../../consts';

const ALICE_ACCOUNT_ADDRESS = '0xd43593c715fdd31c61141abd04a99fd6822c8558854ccde39a5684e7a56da27d';

function useVaraFTAllowance(address: HexString | undefined) {
  const { account, isAccountReady } = useAccount();

  const { data: program } = useVFTProgram(address);

  return useProgramQuery({
    program,
    serviceName: 'vft',
    functionName: 'allowance',
    args: [account?.decodedAddress || ALICE_ACCOUNT_ADDRESS, CONTRACT_ADDRESS.VFT_MANAGER],
    query: { enabled: isAccountReady },
    watch: true,
  });
}

export { useVaraFTAllowance };
