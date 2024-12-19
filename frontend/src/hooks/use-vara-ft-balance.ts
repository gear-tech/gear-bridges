import { HexString } from '@gear-js/api';
import { useAccount, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { VftProgram } from '@/consts';

function useVaraFTBalance(address: HexString | undefined) {
  const { account } = useAccount();

  const { data: program } = useProgram({
    library: VftProgram,
    id: address,
  });

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
