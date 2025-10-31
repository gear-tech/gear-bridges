import { GearApi } from '@gear-js/api';

import { useNetworkType } from '@/context';

function useInitArchiveApi() {
  const { NETWORK_PRESET } = useNetworkType();

  return () => GearApi.create({ providerAddress: NETWORK_PRESET.ARCHIVE_NODE_ADDRESS });
}

export { useInitArchiveApi };
