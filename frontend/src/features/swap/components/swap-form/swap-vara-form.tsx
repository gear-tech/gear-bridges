import { useAccount } from '@gear-js/react-hooks';

import { NETWORK_NAME } from '@/consts';

import { useHandleVaraSubmit, useVaraBalance } from '../../hooks';

import { SwapForm } from './swap-form';

type Props = {
  renderSwapNetworkButton: () => JSX.Element;
};

function SwapVaraForm({ renderSwapNetworkButton }: Props) {
  const { account } = useAccount();

  return (
    <SwapForm
      disabled={!account}
      useHandleSubmit={useHandleVaraSubmit}
      useBalance={useVaraBalance}
      networkName={NETWORK_NAME.VARA}
      renderSwapNetworkButton={renderSwapNetworkButton}
    />
  );
}

export { SwapVaraForm };
