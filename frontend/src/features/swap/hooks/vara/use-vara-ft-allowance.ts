import { HexString } from '@gear-js/api';
import { useAccount, useProgram, useProgramQuery } from '@gear-js/react-hooks';

import { BRIDGING_PAYMENT_CONTRACT_ADDRESS, VftProgram } from '@/consts';

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
    // TODO: remove assertion after @gear-js/react-hooks update to support empty args
    args: [account?.decodedAddress as HexString, BRIDGING_PAYMENT_CONTRACT_ADDRESS],
    query: { enabled: Boolean(account) },
    watch: true,
  });
}

export { useVaraFTAllowance };
