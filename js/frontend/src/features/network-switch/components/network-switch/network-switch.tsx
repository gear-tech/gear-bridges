import { useNetworkType } from '@/context';

import { Dropdown } from '../dropdown';

function NetworkSwitch() {
  const { networkType, switchNetworks } = useNetworkType();

  return <Dropdown value={networkType} onChange={switchNetworks} />;
}

export { NetworkSwitch };
