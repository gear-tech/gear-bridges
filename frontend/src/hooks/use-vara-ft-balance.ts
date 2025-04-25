import { HexString } from '@gear-js/api';
import { useAccount, useProgramQuery } from '@gear-js/react-hooks';

import { useVFTProgram } from './use-vft-program';

function useVaraFTBalance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useVFTProgram(address);

  return useProgramQuery({
    program,
    serviceName: 'vft',
    functionName: 'balanceOf',
    args: [account?.decodedAddress || '0x00'],
    query: { enabled: Boolean(account) },
    watch: true,
  });
}

export { useVaraFTBalance };
