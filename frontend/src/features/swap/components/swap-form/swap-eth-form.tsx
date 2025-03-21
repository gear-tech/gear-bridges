import { JSX } from 'react';

import { ETH_CHAIN_ID } from '@/consts';
import { useEthAccount, useEthAccountBalance } from '@/hooks';

import { NETWORK_INDEX } from '../../consts';
import { useEthFTBalance, useHandleEthSubmit, useEthFee, useEthFTAllowance } from '../../hooks';

import { SwapForm } from './swap-form';

type Props = {
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapEthForm({ renderSwapNetworkButton }: Props) {
  const ethAccount = useEthAccount();
  const isSupportedChain = ethAccount.chainId === ETH_CHAIN_ID;

  return (
    <SwapForm
      networkIndex={NETWORK_INDEX.ETH}
      disabled={!ethAccount.isConnected || !isSupportedChain}
      useHandleSubmit={useHandleEthSubmit}
      useAccountBalance={useEthAccountBalance}
      useFTBalance={useEthFTBalance}
      useFTAllowance={useEthFTAllowance}
      useFee={useEthFee}
      renderSwapNetworkButton={renderSwapNetworkButton}
    />
  );
}

export { SwapEthForm };
