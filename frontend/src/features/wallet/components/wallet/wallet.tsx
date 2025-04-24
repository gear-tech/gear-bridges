import { useAccount, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { isUndefined } from '@polkadot/util';
import { useAppKit, useWalletInfo } from '@reown/appkit/react';

import { FormattedBalance, Skeleton, TruncatedText } from '@/components';
import { useEthAccount, useEthAccountBalance, useModal, useVaraAccountBalance, useVaraSymbol } from '@/hooks';

import WalletSVG from '../../assets/wallet.svg?react';
import { WALLET_SVGS } from '../../consts';
import { NetworkWalletModal } from '../network-wallet-modal';

import styles from './wallet.module.scss';

function Wallet() {
  const { api } = useApi();
  const { account, isAccountReady } = useAccount();
  const varaAccountBalance = useVaraAccountBalance();
  const varaSymbol = useVaraSymbol();
  const SVG = account ? WALLET_SVGS[account.meta.source as keyof typeof WALLET_SVGS] : undefined;

  const ethAccount = useEthAccount();
  const ethAccountBalance = useEthAccountBalance();
  const { walletInfo: ethWallet } = useWalletInfo();

  const [isModalOpen, openModal, closeModal] = useModal();
  const [isSubstrateModalOpen, openSubstrateModal, closeSubstrateModal] = useModal();
  const { open: openEthModal } = useAppKit();

  const handleButtonClick = () => {
    if (account) return openSubstrateModal();
    if (ethAccount.address) return openEthModal();
  };

  // it's probably worth to check isConnecting too, but there is a bug:
  // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
  if (!isAccountReady || ethAccount.isReconnecting || !api || !varaSymbol)
    return <Skeleton width="11rem" height="2rem" />;

  const isConnected = Boolean(account || ethAccount.address);
  const balance = account ? varaAccountBalance : ethAccountBalance;
  const decimals = account ? api.registry.chainDecimals[0] : 18;
  const symbol = account ? varaSymbol : 'ETH';

  return (
    <>
      {isConnected ? (
        <div className={styles.wallet}>
          {!isUndefined(balance.data) ? (
            <div className={styles.balance}>
              <WalletSVG />
              <FormattedBalance value={balance.data} decimals={decimals} symbol={symbol} />
            </div>
          ) : (
            <Skeleton width="9rem" />
          )}

          <button type="button" className={styles.button} onClick={handleButtonClick}>
            {SVG && <SVG />}
            {ethWallet && <img src={ethWallet.icon} alt="wallet" />}

            {account && <TruncatedText value={account.address} />}
            {ethAccount.address && <TruncatedText value={ethAccount.address} />}
          </button>
        </div>
      ) : (
        <Button text="Connect Wallet" size="x-small" onClick={openModal} />
      )}

      {isModalOpen && <NetworkWalletModal close={closeModal} />}
      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { Wallet };
