import { useAccount } from '@gear-js/react-hooks';

import { Skeleton } from '@/components';
import { useEthAccount } from '@/hooks';
import { cx } from '@/utils';

import { Balance } from '../balance';
import { ConnectWalletButton } from '../connect-wallet-button';
import { ConnectedWalletButton } from '../connected-wallet-button';

import styles from './wallet.module.scss';

type Props = {
  className?: string;
};

function Wallet({ className }: Props) {
  const { account, isAccountReady } = useAccount();
  const ethAccount = useEthAccount();

  // it's probably worth to check isConnecting too, but there is a bug:
  // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
  if (!isAccountReady || ethAccount.isReconnecting)
    return <Skeleton width="11rem" height="2rem" className={className} />;

  return (
    <div className={cx(styles.container, className)}>
      <div className={styles.wallet}>
        {account ? (
          <>
            <Balance.Vara />
            <ConnectedWalletButton.Vara />
          </>
        ) : (
          <ConnectWalletButton.Vara />
        )}
      </div>

      <div className={styles.wallet}>
        {ethAccount.address ? (
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
