import { useAccount } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { useWeb3Modal } from '@web3modal/wagmi/react';

import { useEthAccount, useModal } from '@/hooks';

import SwapSVG from '../../assets/swap.svg?react';
import { WalletModal } from '../wallet-modal';

import styles from './swap-network-button.module.scss';

type Props = {
  onClick: () => void;
};

function SwapNetworkButton({ onClick }: Props) {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();
  const { open: openEthModal } = useWeb3Modal();

  const handleClick = () => {
    if (ethAccount.isConnected) return openSubstrateModal();
    if (account) return openEthModal();

    onClick();
  };

  return (
    <>
      <Button icon={SwapSVG} color="grey" className={styles.button} onClick={handleClick} />

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { SwapNetworkButton };