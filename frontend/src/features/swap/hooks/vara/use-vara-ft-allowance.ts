import { HexString } from '@gear-js/api';
import { useAccount, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { VFT_MANAGER_CONTRACT_ADDRESS, VftProgram } from '@/consts';

function useVaraFTAllowance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useProgram({
    library: VftProgram,
    id: address,
  });

  return useProgramQuery({
    program,
    serviceName: 'vft',
    functionName: 'allowance',
    // TODO: get rid of assertions after @gear-js/react-hooks update to support empty args
    args: [account?.decodedAddress as HexString, VFT_MANAGER_CONTRACT_ADDRESS],
    query: { enabled: Boolean(account) },
    watch: true,
  });
}

export { useVaraFTAllowance };
