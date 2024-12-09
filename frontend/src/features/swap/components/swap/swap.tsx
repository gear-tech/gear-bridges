import { useAccount } from '@gear-js/react-hooks';
import { useMemo, useState } from 'react';

import { useEthAccount } from '@/hooks';
import { cx } from '@/utils';

import { SwapEthForm, SwapVaraForm } from '../swap-form';

import styles from './swap.module.scss';

type Props = {
  renderSwapNetworkButton: (onClick: () => void) => JSX.Element;
};

function Swap({ renderSwapNetworkButton }: Props) {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const [isEthNetwork, setIsEthNetwork] = useState(ethAccount.isConnected);

  const Form = useMemo(() => {
    // since eth account is reconnecting immediately without any visible loading state,
    // and in swap form vara is the first network by default,
    // check for loading status (isAccountReady || ethAccount.isReconnecting) is minor and can be neglected
    if (ethAccount.isConnected) return SwapEthForm;
    if (account) return SwapVaraForm;

    return isEthNetwork ? SwapEthForm : SwapVaraForm;
  }, [isEthNetwork, ethAccount, account]);

  return (
    <div className={cx(styles.card, (account || ethAccount.isConnected) && styles.active)}>
      <Form renderSwapNetworkButton={() => renderSwapNetworkButton(() => setIsEthNetwork((prevValue) => !prevValue))} />
    </div>
  );
}

export { Swap };
