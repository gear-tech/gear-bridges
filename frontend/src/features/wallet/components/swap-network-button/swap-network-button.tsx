import { useAccount } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { useAppKit } from '@reown/appkit/react';

import { useEthAccount, useModal } from '@/hooks';
import { cx } from '@/utils';

import SwapSVG from '../../assets/swap.svg?react';
import { WalletModal } from '../wallet-modal';

import styles from './swap-network-button.module.scss';

type Props = {
  isActive: boolean;
  onClick: () => void;
};

function SwapNetworkButton({ isActive, onClick }: Props) {
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
      <Button
        icon={SwapSVG}
        color="contrast"
        className={cx(styles.button, isActive && styles.active)}
        onClick={handleClick}
      />

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { SwapNetworkButton };
