import { HexString } from '@gear-js/api';
import { useAccount, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { VftProgram } from '@/consts';

import { useVFTManagerAddress } from './use-vft-manager-address';

function useVaraFTAllowance(address: HexString | undefined) {
  const { account } = useAccount();
  const { data: vftManagerAddress } = useVFTManagerAddress();

  const { data: program } = useProgram({
    library: VftProgram,
    id: address,
  });

  return useProgramQuery({
    program,
    serviceName: 'vft',
    functionName: 'allowance',
    // TODO: get rid of assertions after @gear-js/react-hooks update to support empty args
    args: [account?.decodedAddress as HexString, vftManagerAddress!],
    query: { enabled: Boolean(account && vftManagerAddress) },
    watch: true,
  });
}

export { useVaraFTAllowance };
