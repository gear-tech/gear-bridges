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
  const { extensions, account, login, logout } = useAccount();
  const { wallet, walletAccounts, setWalletId, resetWalletId, getWalletAccounts } = useWallet();

  const getWallets = () =>
    WALLETS.map(([id, { SVG, name }]) => {
      const isEnabled = extensions?.some((extension) => extension.name === id);
      const status = isEnabled ? 'Enabled' : 'Disabled';

      const accountsCount = getWalletAccounts(id)?.length;
      const accountsStatus = `${accountsCount} ${accountsCount === 1 ? 'account' : 'accounts'}`;

      const onClick = () => setWalletId(id);

      return (
        <li key={id}>
          <Button
            className={styles.walletButton}
            color="light"
            size="small"
            onClick={onClick}
            disabled={!isEnabled}
            block>
            <WalletItem SVG={SVG} name={name} />

            <span className={styles.status}>
              <p className={styles.statusText}>{status}</p>

              {isEnabled && <p className={styles.statusAccounts}>{accountsStatus}</p>}
            </span>
          </Button>
        </li>
      );
    });

  const getAccounts = () =>
    walletAccounts?.map((_account) => {
      const { address, meta } = _account;

      const isActive = address === account?.address;
      const color = isActive ? 'primary' : 'light';

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

  return (
    <Modal
      heading="Connect Wallet"
      close={close}
      footer={
        wallet ? (
          <div className={styles.footer}>
            <Button color="transparent" onClick={resetWalletId}>
              <WalletItem SVG={wallet.SVG} name={wallet.name} />

              <span className={styles.changeText}>Change</span>
            </Button>

            {account && <Button icon={ExitSVG} text="Logout" color="transparent" onClick={handleLogoutButtonClick} />}
          </div>
        ) : null
      }>
      {extensions?.length ? (
        <ul className={styles.list}>{getAccounts() || getWallets()}</ul>
      ) : (
        <div className={styles.instruction}>
          <p>A compatible wallet wasn&apos;t found or is disabled.</p>
          <p>
            Please, install it following the{' '}
            <a href="https://wiki.vara-network.io/docs/account/create-account/" target="_blank" rel="noreferrer">
              instructions
            </a>
            .
          </p>
        </div>
      )}
    </Modal>
  );
}

export { WalletModal };
