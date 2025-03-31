import { useAccount } from '@gear-js/react-hooks';

import { useVaraFTBalance, useVaraAccountBalance } from '@/hooks';

import { useHandleVaraSubmit, useVaraFee, useVaraFTAllowance } from '../../hooks';

import { SwapForm } from './swap-form';

function SwapVaraForm() {
  const { account } = useAccount();

  return (
    <SwapForm
      disabled={!account}
      useHandleSubmit={useHandleVaraSubmit}
      useAccountBalance={useVaraAccountBalance}
      useFTBalance={useVaraFTBalance}
      useFTAllowance={useVaraFTAllowance}
      useFee={useVaraFee}
    />
  );
}

export { SwapVaraForm };
