import { Sails } from 'sails-js';
import { HexString } from '@gear-js/api';
import { useAccount } from '@gear-js/react-hooks';
import { useQuery } from '@tanstack/react-query';

import { isUndefined } from '@/utils';

function useReadState<T>(programId: HexString | undefined, sails: Sails | undefined, queryName: string) {
  const { account } = useAccount();

  const isEnabled = programId && account && sails && !isUndefined(queryName);

  return useQuery({
    queryKey: ['readState', isEnabled, programId, queryName],
    queryFn: async () => {
      if (!isEnabled) {
        return null;
      }
      sails.setProgramId(programId);
      return (await sails.services.VaraBridge.queries.Config(account.decodedAddress)) as T;
    },
    enabled: isEnabled,
  });
}

export { useReadState };
