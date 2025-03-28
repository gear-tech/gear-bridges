import { useAccount } from '@gear-js/react-hooks';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import { useEthAccount, useModal } from '@/hooks';

import SwapSVG from '../../assets/swap.svg?react';

import styles from './swap-network-button.module.scss';

type Props = {
  onClick: () => void;
};

function SwapNetworkButton({ onClick }: Props) {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();
  const { open: openEthModal } = useAppKit();

  const handleClick = () => {
    if (ethAccount.isConnected) return openSubstrateModal();
    if (account) return openEthModal();

    onClick();
  };

  return (
    <>
      <button type="button" color="contrast" className={styles.button} onClick={handleClick}>
        <SwapSVG className={styles.icon} />
      </button>

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { SwapNetworkButton };
