import { useAccount } from '@gear-js/react-hooks';

import CheckSVG from '@/assets/check.svg?react';
import { useEthAccount } from '@/hooks';

import WalletSVG from '../../assets/wallet.svg?react';
import { ADDRESS_SCHEMA } from '../../consts';
import { useBridgeContext } from '../../context';

import styles from './wallet-address-button.module.scss';

type Props = {
  value: string;
  onClick: (value: string) => void;
};

function WalletAddressButton({ value, onClick }: Props) {
  const { network } = useBridgeContext();
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const schema = network.isVara ? ADDRESS_SCHEMA.ETH : ADDRESS_SCHEMA.VARA;
  const address = network.isVara ? ethAccount.address : account?.address;

  if (!address) return;

  const isActive = schema.safeParse(value).data === schema.safeParse(address).data;
  const SVG = isActive ? CheckSVG : WalletSVG;

  return (
    <button type="button" className={styles.addressButton} onClick={() => onClick(isActive ? '' : address)}>
      <SVG />
      {isActive ? 'Using' : 'Use'} Wallet Address
    </button>
  );
}

export { WalletAddressButton };
