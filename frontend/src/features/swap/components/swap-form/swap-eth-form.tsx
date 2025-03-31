import { ETH_CHAIN_ID } from '@/consts';
import { useEthAccount, useEthAccountBalance } from '@/hooks';

import { useEthFTBalance, useHandleEthSubmit, useEthFee, useEthFTAllowance } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapEthForm() {
  const ethAccount = useEthAccount();
  const isSupportedChain = ethAccount.chainId === ETH_CHAIN_ID;

  return (
    <SwapForm
      disabled={!ethAccount.isConnected || !isSupportedChain}
      useHandleSubmit={useHandleEthSubmit}
      useAccountBalance={useEthAccountBalance}
      useFTBalance={useEthFTBalance}
      useFTAllowance={useEthFTAllowance}
      useFee={useEthFee}
    />
  );
}

export { SwapEthForm };
