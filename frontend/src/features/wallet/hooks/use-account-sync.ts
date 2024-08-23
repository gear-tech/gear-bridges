import { useAccount } from '@gear-js/react-hooks';
import { useEffect } from 'react';
import { useDisconnect } from 'wagmi';

import { useEthAccount } from '@/hooks';

function useAccountSync() {
  const { account, logout } = useAccount();

  const ethAccount = useEthAccount();
  const { disconnect } = useDisconnect();

  useEffect(() => {
    // eth wallet can be connected solely by the extension
    if (!ethAccount.isConnected) return;

    logout();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ethAccount.isConnected]);

  useEffect(() => {
    // wallet can be connected via swap network button
    if (!account) return;

    disconnect();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account]);
}

export { useAccountSync };
