import { useVaraFTBalance, useVaraAccountBalance } from '@/hooks';

import { useHandleVaraSubmit, useVaraFee } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapVaraForm() {
  return (
    <SwapForm
      useHandleSubmit={useHandleVaraSubmit}
      useAccountBalance={useVaraAccountBalance}
      useFTBalance={useVaraFTBalance}
      useFee={useVaraFee}
    />
  );
}

export { SwapVaraForm };
