import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { useModal } from '@/hooks';
import { PropsWithClassName, SVGComponent } from '@/types';
import { cx } from '@/utils';

import styles from './connect-wallet-button.module.scss';

type Props = PropsWithClassName & {
  icon: SVGComponent;
  onClick: () => void;
};

function ConnectButton({ icon: Icon, className, onClick }: Props) {
  return (
    <Button size="x-small" className={cx(styles.button, className)} onClick={() => onClick()}>
      <Icon />
      <span>Connect</span>
    </Button>
  );
}

function ConnectVaraWalletButton(props: PropsWithClassName) {
  const [isModalOpen, openModal, closeModal] = useModal();

  return (
    <>
      <ConnectButton icon={VaraSVG} onClick={openModal} {...props} />

      {isModalOpen && <WalletModal close={closeModal} />}
    </>
  );
}

function ConnectEthWalletButton(props: PropsWithClassName) {
  const { open } = useAppKit();

  return <ConnectButton icon={EthSVG} onClick={open} {...props} />;
}

const ConnectWalletButton = {
  Vara: ConnectVaraWalletButton,
  Eth: ConnectEthWalletButton,
};

export { ConnectWalletButton };
