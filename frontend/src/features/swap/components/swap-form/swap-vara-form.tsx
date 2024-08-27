import { useAccount } from '@gear-js/react-hooks';

import { NETWORK_INDEX } from '../../consts';
import { useHandleVaraSubmit, useVaraBalance } from '../../hooks';

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
      useBalance={useVaraBalance}
      renderSwapNetworkButton={renderSwapNetworkButton}
    />
  );
}

export { SwapVaraForm };
