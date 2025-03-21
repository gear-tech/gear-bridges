import { useAccount } from '@gear-js/react-hooks';
import { Button, Modal } from '@gear-js/vara-ui';

import { CopyButton } from '@/components';

import CopySVG from '../../assets/copy.svg?react';
import ExitSVG from '../../assets/exit.svg?react';
import { WALLETS } from '../../consts';
import { useWallet } from '../../hooks';
import { AccountButton } from '../account-button';
import { WalletItem } from '../wallet-item';

import styles from './wallet-modal.module.scss';

type Props = {
  close: () => void;
};

function WalletModal({ close }: Props) {
  const { wallets, isAnyWallet, account, login, logout } = useAccount();
  const { wallet, walletAccounts, setWalletId, resetWalletId } = useWallet();

  const getWallets = () =>
    WALLETS.map(([id, { SVG, name }]) => {
      const { status, accounts, connect } = wallets?.[id] || {};
      const isEnabled = Boolean(status);
      const isConnected = status === 'connected';

      const accountsCount = accounts?.length;
      const accountsStatus = `${accountsCount} ${accountsCount === 1 ? 'account' : 'accounts'}`;

      return (
        <li key={id}>
          <Button
            className={styles.walletButton}
            color="contrast"
            size="small"
            onClick={() => (isConnected ? setWalletId(id) : connect?.())}
            disabled={!isEnabled}
            block>
            <WalletItem SVG={SVG} name={name} />

            <span className={styles.status}>
              <p className={styles.statusText}>{isConnected ? 'Enabled' : 'Disabled'}</p>

              {isConnected && <p className={styles.statusAccounts}>{accountsStatus}</p>}
            </span>
          </Button>
        </li>
      );
    });

  const getAccounts = () =>
    walletAccounts?.map((_account) => {
      const { address, meta } = _account;

      const isActive = address === account?.address;
      const color = isActive ? 'primary' : 'contrast';

      const handleClick = () => {
        if (isActive) return;

        login(_account);
        close();
      };

      return (
        <li key={address} className={styles.account}>
          <AccountButton size="small" address={address} name={meta.name} color={color} onClick={handleClick} block />
          <CopyButton SVG={CopySVG} value={address} onCopy={close} />
        </li>
      );
    });

  const handleLogoutButtonClick = () => {
    logout();
    close();
  };

  const renderFooter = () => {
    if (!wallet) return;

    return (
      <div className={styles.footer}>
        <Button color="transparent" onClick={resetWalletId}>
          <WalletItem SVG={wallet.SVG} name={wallet.name} />

          <span className={styles.changeText}>Change</span>
        </Button>

        {account && <Button icon={ExitSVG} text="Logout" color="transparent" onClick={handleLogoutButtonClick} />}
      </div>
    );
  };

  const render = () => {
    if (!isAnyWallet)
      return (
        <div className={styles.instruction}>
          <p>A compatible wallet wasn&apos;t found or is disabled.</p>
          <p>
            Please, install it following the{' '}
            <a href="https://wiki.vara.network/docs/account/" target="_blank" rel="noreferrer">
              instructions
            </a>
            .
          </p>
        </div>
      );

    if (!walletAccounts) return <ul className={styles.list}>{getWallets()}</ul>;

    if (walletAccounts.length) return <ul className={styles.list}>{getAccounts()}</ul>;

    return <p>No accounts found. Please open your extension and create a new account or import existing.</p>;
  };

  return (
    <Modal heading="Connect Wallet" close={close} footer={renderFooter()}>
      {render()}
    </Modal>
  );
}

export { WalletModal };
