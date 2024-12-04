import { useAccount } from '@gear-js/react-hooks';

import { useVaraFTBalance, useVaraAccountBalance } from '@/hooks';

import { NETWORK_INDEX } from '../../consts';
import { useHandleVaraSubmit, useVaraFee, useVaraFTAllowance } from '../../hooks';

import { SwapForm } from './swap-form';

type Props = {
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapVaraForm({ renderSwapNetworkButton }: Props) {
  const { account } = useAccount();

  return (
    <SwapForm
      networkIndex={NETWORK_INDEX.VARA}
      disabled={!account}
      useHandleSubmit={useHandleVaraSubmit}
      useAccountBalance={useVaraAccountBalance}
      useFTBalance={useVaraFTBalance}
      useFTAllowance={useVaraFTAllowance}
      useFee={useVaraFee}
      renderSwapNetworkButton={renderSwapNetworkButton}
    />
  );
}

export { SwapVaraForm };
