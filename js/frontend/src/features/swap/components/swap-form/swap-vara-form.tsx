import { useVaraFTBalance, useVaraAccountBalance } from '@/hooks';

import { useSendVaraTxs, useVaraFee, useVaraTxsEstimate } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapVaraForm() {
  return (
    <SwapForm
      useAccountBalance={useVaraAccountBalance}
      useFTBalance={useVaraFTBalance}
      useFee={useVaraFee}
      useSendTxs={useSendVaraTxs}
      useTxsEstimate={useVaraTxsEstimate}
    />
  );
}

export { SwapVaraForm };
