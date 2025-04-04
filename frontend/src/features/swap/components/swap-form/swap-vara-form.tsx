import { useVaraFTBalance, useVaraAccountBalance } from '@/hooks';

import { useHandleVaraSubmit, useVaraFee, useVaraFTAllowance } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapVaraForm() {
  return (
    <SwapForm
      useHandleSubmit={useHandleVaraSubmit}
      useAccountBalance={useVaraAccountBalance}
      useFTBalance={useVaraFTBalance}
      useFTAllowance={useVaraFTAllowance}
      useFee={useVaraFee}
    />
  );
}

export { SwapVaraForm };
