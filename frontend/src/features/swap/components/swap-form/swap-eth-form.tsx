import { ETH_CHAIN_ID, NETWORK_NAME } from '@/consts';
import { useEthAccount } from '@/hooks';

import { useEthBalance, useHandleEthSubmit } from '../../hooks';

import { SwapForm } from './swap-form';

type Props = {
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapEthForm({ renderSwapNetworkButton }: Props) {
  const ethAccount = useEthAccount();
  const isSupportedChain = ethAccount.chainId === ETH_CHAIN_ID;

  return (
    <SwapForm
      networkName={NETWORK_NAME.ETH}
      disabled={!ethAccount.isConnected || !isSupportedChain}
      useHandleSubmit={useHandleEthSubmit}
      useBalance={useEthBalance}
      renderSwapNetworkButton={renderSwapNetworkButton}
    />
  );
}

export { SwapEthForm };
