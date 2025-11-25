import { Skeleton } from '@/components';
import { useAccountsConnection } from '@/hooks';
import { PropsWithClassName } from '@/types';
import { cx } from '@/utils';

import { Balance } from '../balance';
import { ConnectWalletButton } from '../connect-wallet-button';
import { ConnectedWalletButton } from '../connected-wallet-button';

import styles from './wallet.module.scss';

function Wallet({ className }: PropsWithClassName) {
  const { isAnyAccountLoading, isVaraAccount, isEthAccount } = useAccountsConnection();

  if (isAnyAccountLoading)
    return (
      <div className={cx(styles.container, className)}>
        <Skeleton height="2rem" className={styles.skeleton} />
        <Skeleton height="2rem" className={styles.skeleton} />
      </div>
    );

  return (
    <div className={cx(styles.container, className)}>
      <div className={styles.wallet}>
        {isVaraAccount ? (
          <>
            <Balance.Vara />
            <ConnectedWalletButton.Vara />
          </>
        ) : (
          <ConnectWalletButton.Vara />
        )}
      </div>

      <div className={styles.wallet}>
        {isEthAccount ? (
          <>
            <Balance.Eth />
            <ConnectedWalletButton.Eth />
          </>
        ) : (
          <ConnectWalletButton.Eth />
        )}
      </div>
    </div>
  );
}

export { Wallet };
