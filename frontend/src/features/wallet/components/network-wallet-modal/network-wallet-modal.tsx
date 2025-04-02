import { useAccount } from '@gear-js/react-hooks';
import { Modal, ModalProps } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';
import { useEffect } from 'react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { useEthAccount, useModal } from '@/hooks';

import styles from './network-wallet-modal.module.scss';

type Props = Pick<ModalProps, 'close'>;

function NetworkWalletModal({ close }: Props) {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();
  const { open: openEthModal } = useAppKit();

  useEffect(() => {
    if (!account && !ethAccount.isConnected) return;

    close();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account, ethAccount.isConnected]);

  return (
    <>
      <Modal heading="Connect Wallet" close={close} className={styles.modal}>
        <button type="button" onClick={openSubstrateModal} className={styles.button}>
          <VaraSVG />
          <span>Substrate</span>
        </button>

        <button type="button" onClick={() => openEthModal()} className={styles.button}>
          <EthSVG />
          <span>Ethereum</span>
        </button>
      </Modal>

      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { NetworkWalletModal };
