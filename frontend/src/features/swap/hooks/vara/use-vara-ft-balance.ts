import { HexString } from '@gear-js/api';
import { useAccount, useProgram, useProgramQuery } from '@gear-js/react-hooks';
import { formatUnits } from 'viem';

import { VftProgram } from '@/consts';
import { isUndefined } from '@/utils';

import { QUERY_NAME, SERVICE_NAME } from '../../consts';

function useVaraFTBalance(address: HexString | undefined, decimals: number | undefined) {
  const { account } = useAccount();

  const { data: program } = useProgram({
    library: VftProgram,
    id: address,
  });

  const { data, isLoading } = useProgramQuery({
    program,
    serviceName: SERVICE_NAME.VFT,
    functionName: QUERY_NAME.BALANCE,
    args: [account?.decodedAddress || '0x00'],
    query: { enabled: Boolean(account) },
    watch: true,
  });

  const value = data;
  const formattedValue = !isUndefined(value) && !isUndefined(decimals) ? formatUnits(value, decimals) : undefined;

  return { value, formattedValue, decimals, isLoading };
}

export { useVaraFTBalance };
