import { HexString, ProgramMetadata } from '@gear-js/api';
import { useApi } from '@gear-js/react-hooks';
import { AnyJson } from '@polkadot/types/types';
import { useQuery } from '@tanstack/react-query';

import { isUndefined } from '@/utils';

function useReadState<T>(programId: HexString | undefined, metadata: ProgramMetadata | undefined, payload: AnyJson) {
  const { api, isApiReady } = useApi();

  const isEnabled = isApiReady && programId && metadata && !isUndefined(payload);

  return useQuery({
    queryKey: ['readState', isApiReady, programId, metadata, payload],
    queryFn: async () =>
      isEnabled ? ((await api.programState.read({ programId, payload }, metadata)).toHuman() as T) : null,
    enabled: isEnabled,
  });
}

export { useReadState };
