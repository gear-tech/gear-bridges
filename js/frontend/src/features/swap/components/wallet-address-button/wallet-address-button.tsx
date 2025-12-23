import CheckSVG from '@/assets/check.svg?react';

import WalletSVG from '../../assets/wallet.svg?react';

import styles from './wallet-address-button.module.scss';

type Props = {
  isActive: boolean;
  onClick: () => void;
};

function WalletAddressButton({ isActive, onClick }: Props) {
  const SVG = isActive ? CheckSVG : WalletSVG;

  return (
    <button type="button" className={styles.addressButton} onClick={onClick}>
      <SVG />
      Use Wallet Address
    </button>
  );
}

export { WalletAddressButton };
