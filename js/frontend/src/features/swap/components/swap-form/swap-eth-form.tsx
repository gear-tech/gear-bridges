import { useEthAccountBalance, useEthFTBalance } from '@/hooks';

import { useHandleEthSubmit, useEthFee } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapEthForm() {
  return (
    <SwapForm
      useHandleSubmit={useHandleEthSubmit}
      useAccountBalance={useEthAccountBalance}
      useFTBalance={useEthFTBalance}
      useFee={useEthFee}
    />
  );
}

export { SwapEthForm };
