import { useEthAccountBalance, useEthFTBalance } from '@/hooks';

import { useEthFee, useSendEthTxs, useEthTxsEstimate } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapEthForm() {
  return (
    <SwapForm
      useAccountBalance={useEthAccountBalance}
      useFTBalance={useEthFTBalance}
      useFee={useEthFee}
      useSendTxs={useSendEthTxs}
      useTxsEstimate={useEthTxsEstimate}
    />
  );
}

export { SwapEthForm };
