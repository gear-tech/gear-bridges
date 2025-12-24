import { useAccount } from '@gear-js/react-hooks';
import { useMemo } from 'react';

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

  const normalizedAddress = useMemo(() => (address ? schema.safeParse(address).data : undefined), [address, schema]);

  const normalizedValue = useMemo(
    () => (value && address ? schema.safeParse(value).data : undefined),
    [value, address, schema],
  );

  if (!address || !normalizedAddress) return;

  const isActive = normalizedAddress === normalizedValue;
  const SVG = isActive ? CheckSVG : WalletSVG;

  return (
    <button type="button" className={styles.addressButton} onClick={() => onClick(isActive ? '' : address)}>
      <SVG />
      {isActive ? 'Using' : 'Use'} Wallet Address
    </button>
  );
}

export { WalletAddressButton };
