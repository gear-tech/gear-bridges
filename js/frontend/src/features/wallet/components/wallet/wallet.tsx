import { Skeleton } from '@/components';
import { useAccountsConnection } from '@/hooks';
import { cx } from '@/utils';

import { Balance } from '../balance';
import { ConnectWalletButton } from '../connect-wallet-button';
import { ConnectedWalletButton } from '../connected-wallet-button';

import styles from './wallet.module.scss';

type Props = {
  className?: string;
};

function Wallet({ className }: Props) {
  const { isAnyAccountLoading, isVaraAccount, isEthAccount } = useAccountsConnection();

  if (isAnyAccountLoading) return <Skeleton width="11rem" height="2rem" className={className} />;

  return (
    <div className={cx(styles.container, className)}>
      <div className={styles.wallet}>
        {isVaraAccount ? (
          <>
            <Balance.Vara className={styles.balance} />
            <ConnectedWalletButton.Vara />
          </>
        ) : (
          <ConnectWalletButton.Vara className={styles.connectButton} />
        )}
      </div>

      <div className={styles.wallet}>
        {isEthAccount ? (
          <>
            <Balance.Eth className={styles.balance} />
            <ConnectedWalletButton.Eth />
          </>
        ) : (
          <ConnectWalletButton.Eth className={styles.connectButton} />
        )}
      </div>
    </div>
  );
}

export { Wallet };
