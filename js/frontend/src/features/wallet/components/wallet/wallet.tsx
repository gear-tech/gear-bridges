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
