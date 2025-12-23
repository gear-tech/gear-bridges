import WalletSVG from '../../assets/wallet.svg?react';

import styles from './wallet-address-button.module.scss';

type Props = {
  onClick: () => void;
};

function WalletAddressButton({ onClick }: Props) {
  return (
    <button type="button" className={styles.addressButton} onClick={onClick}>
      <WalletSVG />
      Use Wallet Address
    </button>
  );
}

export { WalletAddressButton };
