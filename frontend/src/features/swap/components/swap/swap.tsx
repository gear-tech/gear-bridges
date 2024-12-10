import { useAccount } from '@gear-js/react-hooks';
import { useMemo, useState } from 'react';

import { ETH_CHAIN_ID } from '@/consts';
import { SwapNetworkButton } from '@/features/wallet';
import { useEthAccount } from '@/hooks';

import { SwapEthForm, SwapVaraForm } from '../swap-form';

function Swap() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const [isEthNetwork, setIsEthNetwork] = useState(ethAccount.isConnected);

  const Form = useMemo(() => {
    // since eth account is reconnecting immediately without any visible loading state,
    // and in swap form vara is the first network by default,
    // check for loading status (isAccountReady || ethAccount.isReconnecting) is minor and can be neglected
    if (ethAccount.isConnected) return SwapEthForm;
    if (account) return SwapVaraForm;

    return isEthNetwork ? SwapEthForm : SwapVaraForm;
  }, [isEthNetwork, ethAccount, account]);

  return (
    <Form
      renderSwapNetworkButton={() => (
        <SwapNetworkButton
          onClick={() => setIsEthNetwork((prevValue) => !prevValue)}
          isActive={(ethAccount.isConnected && ethAccount.chainId === ETH_CHAIN_ID) || Boolean(account)}
        />
      )}
    />
  );
}

export { Swap };
