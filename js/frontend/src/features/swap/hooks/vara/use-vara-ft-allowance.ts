import { HexString } from '@gear-js/api';
import { useAccount, useProgramQuery } from '@gear-js/react-hooks';

import { useVFTProgram } from '@/hooks';

import { CONTRACT_ADDRESS } from '../../consts';

function useVaraFTAllowance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useVFTProgram(address);

  return useProgramQuery({
    program,
    serviceName: 'vft',
    functionName: 'allowance',
    // TODO: get rid of assertions after @gear-js/react-hooks update to support empty args
    args: [account?.decodedAddress as HexString, CONTRACT_ADDRESS.VFT_MANAGER],
    query: { enabled: Boolean(account) },
    watch: true,
  });
}

export { useVaraFTAllowance };
