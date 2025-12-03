import { useAccount } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit, useWalletInfo } from '@reown/appkit/react';

import EthSVG from '@/assets/eth.svg?react';
import { useEthAccount, useModal } from '@/hooks';
import { SVGComponent } from '@/types';
import { getTruncatedText } from '@/utils';

import { WALLET_SVGS } from '../../consts';

import styles from './connected-wallet-button.module.scss';

type Props = {
  icon: SVGComponent | string;
  address: string;
  onClick: () => void;
};

function ConnectedButton({ icon: Icon, address, onClick }: Props) {
  return (
    <Button size="x-small" color="contrast" className={styles.button} onClick={() => onClick()}>
      {typeof Icon === 'string' ? <img src={Icon} alt="wallet icon" /> : <Icon />}
      <span className={styles.address}>{getTruncatedText(address, 4)}</span>
    </Button>
  );
}

function ConnectedVaraWalletButton() {
  const { account } = useAccount();
  const [isModalOpen, openModal, closeModal] = useModal();

  const SVG = account ? WALLET_SVGS[account.meta.source as keyof typeof WALLET_SVGS] : undefined;

  if (!account || !SVG) return;

  return (
    <>
      <ConnectedButton icon={SVG} address={account.address} onClick={openModal} />

      {isModalOpen && <WalletModal close={closeModal} />}
    </>
  );
}

function ConnectedEthWalletButton() {
  const ethAccount = useEthAccount();
  const { walletInfo } = useWalletInfo();
  const { open } = useAppKit();

  if (!ethAccount.address) return;

  return <ConnectedButton icon={walletInfo?.icon || EthSVG} address={ethAccount.address} onClick={open} />;
}

const ConnectedWalletButton = {
  Vara: ConnectedVaraWalletButton,
  Eth: ConnectedEthWalletButton,
};

export { ConnectedWalletButton };
