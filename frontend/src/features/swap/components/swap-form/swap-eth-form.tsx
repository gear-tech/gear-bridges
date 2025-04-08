import { useEthAccountBalance } from '@/hooks';

import { useEthFTBalance, useHandleEthSubmit, useEthFee, useEthFTAllowance } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapEthForm() {
  return (
    <SwapForm
      useHandleSubmit={useHandleEthSubmit}
      useAccountBalance={useEthAccountBalance}
      useFTBalance={useEthFTBalance}
      useFTAllowance={useEthFTAllowance}
      useFee={useEthFee}
    />
  );
}

export { SwapEthForm };
