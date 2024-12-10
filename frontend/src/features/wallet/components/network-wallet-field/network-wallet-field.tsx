import { useAccount } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { useWalletInfo, useWeb3Modal } from '@web3modal/wagmi/react';

import { Skeleton, TruncatedText } from '@/components';
import { useEthAccount, useModal } from '@/hooks';

import { WALLET } from '../../consts';
import { WalletId } from '../../types';
import { NetworkWalletModal } from '../network-wallet-modal';
import { WalletModal } from '../wallet-modal';

import styles from './network-wallet-field.module.scss';

function NetworkWalletField() {
  const { account, isAccountReady } = useAccount();
  const wallet = account ? WALLET[account.meta.source as WalletId] : undefined;
  const { SVG } = wallet || {};

  const ethAccount = useEthAccount();
  const { walletInfo: ethWallet } = useWalletInfo();

  const [isModalOpen, openModal, closeModal] = useModal();
  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();
  const { open: openEthModal } = useWeb3Modal();

  const isConnected = Boolean(account || ethAccount.address);

  const handleButtonClick = () => {
    if (account) return openSubstrateModal();
    if (ethAccount.address) return openEthModal();
  };

  // it's probably worth to check isConnecting too, but there is a bug:
  // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
  if (!isAccountReady || ethAccount.isReconnecting) return <Skeleton height="100%" width="100%" />;

  return (
    <>
      {isConnected ? (
        // TODO: button
        // eslint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/no-static-element-interactions
        <button type="button" className={styles.button} onClick={handleButtonClick}>
          {SVG && <SVG />}
          {ethWallet && <img src={ethWallet.icon} alt="wallet" />}

          {account && <TruncatedText value={account.address} />}
          {ethAccount.address && <TruncatedText value={ethAccount.address} />}
        </button>
      ) : (
        <Button text="Connect" size="small" onClick={openModal} block />
      )}

      {isModalOpen && <NetworkWalletModal close={closeModal} />}
      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { NetworkWalletField };
