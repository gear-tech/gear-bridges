import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { useModal } from '@/hooks';
import { SVGComponent } from '@/types';

import styles from './connect-wallet-button.module.scss';

type Props = {
  icon: SVGComponent;
  onClick: () => void;
};

function ConnectButton({ icon: Icon, onClick }: Props) {
  return (
    <Button size="x-small" onClick={() => onClick()}>
      <Icon className={styles.icon} />
      <span>Connect</span>
    </Button>
  );
}

function ConnectVaraWalletButton() {
  const [isModalOpen, openModal, closeModal] = useModal();

  return (
    <>
      <ConnectButton icon={VaraSVG} onClick={openModal} />

      {isModalOpen && <WalletModal close={closeModal} />}
    </>
  );
}

function ConnectEthWalletButton() {
  const { open } = useAppKit();

  return <ConnectButton icon={EthSVG} onClick={open} />;
}

const ConnectWalletButton = {
  Vara: ConnectVaraWalletButton,
  Eth: ConnectEthWalletButton,
};

export { ConnectWalletButton };
