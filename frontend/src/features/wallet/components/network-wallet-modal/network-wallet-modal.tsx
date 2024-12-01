import { useAccount } from '@gear-js/react-hooks';
import { Button, Modal, ModalProps } from '@gear-js/vara-ui';
import { useWeb3Modal } from '@web3modal/wagmi/react';
import { useEffect } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { useEthAccount, useModal } from '@/hooks';

import { WalletModal } from '../wallet-modal';

import styles from './network-wallet-modal.module.scss';

type Props = Pick<ModalProps, 'close'>;

function NetworkWalletModal({ close }: Props) {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();
  const { open: openEthModal } = useWeb3Modal();

  useEffect(() => {
    if (!account && !ethAccount.isConnected) return;

    close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account, ethAccount.isConnected]);

  return (
    <>
      <Modal heading="Connect Wallet" close={close} className={styles.modal}>
        {/* TODO: NetworkCard */}
        <Button icon={VaraSVG} text="Substrate" onClick={openSubstrateModal} size="small" color="grey" block />
        <Button icon={EthSVG} text="Ethereum" onClick={() => openEthModal()} size="small" color="grey" block />
      </Modal>

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { NetworkWalletModal };
