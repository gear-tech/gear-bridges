import { useNetworkType } from '@/context/network-type';

import { Dropdown } from '../dropdown';

function NetworkSwitch() {
  const { networkType, isLoading, switchNetworks } = useNetworkType();

  return <Dropdown value={networkType} isLoading={isLoading} onChange={switchNetworks} />;
}

export { NetworkSwitch };
