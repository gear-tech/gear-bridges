import { useAccount, useApi } from '@gear-js/react-hooks';
import { Button } from '@gear-js/vara-ui';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit, useWalletInfo } from '@reown/appkit/react';

import EthSVG from '@/assets/eth.svg?react';
import { FormattedBalance, Skeleton } from '@/components';
import { useEthAccount, useEthAccountBalance, useModal, useVaraAccountBalance, useVaraSymbol } from '@/hooks';
import { cx, getTruncatedText, isUndefined } from '@/utils';

import WalletSVG from '../../assets/wallet.svg?react';
import { WALLET_SVGS } from '../../consts';
import { NetworkWalletModal } from '../network-wallet-modal';

import styles from './wallet.module.scss';

type Props = {
  className?: string;
};

function Wallet({ className }: Props) {
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
    return <Skeleton width="11rem" height="2rem" className={className} />;

  const address = account?.address || ethAccount.address;
  const balance = account ? varaAccountBalance : ethAccountBalance;
  const decimals = account ? api.registry.chainDecimals[0] : 18;
  const symbol = account ? varaSymbol : 'ETH';

  return (
    <>
      {address ? (
        <button type="button" className={cx(styles.wallet, className)} onClick={handleButtonClick}>
          {!isUndefined(balance.data) ? (
            <span className={styles.balance}>
              <WalletSVG />
              <FormattedBalance value={balance.data} decimals={decimals} symbol={symbol} />
            </span>
          ) : (
            <Skeleton width="9rem" className={styles.skeleton} />
          )}

          <span className={styles.account}>
            {SVG && <SVG />}

            {/* icon from useWalletInfo only exists on initial wallet connection */}
            {ethWallet?.icon ? <img src={ethWallet.icon} alt="wallet" /> : ethAccount.address && <EthSVG />}

            <span className={styles.address}>{getTruncatedText(address)}</span>
          </span>
        </button>
      ) : (
        <Button text="Connect Wallet" size="x-small" onClick={openModal} className={className} />
      )}

      {isModalOpen && <NetworkWalletModal close={closeModal} />}
      {isSubstrateModalOpen && <WalletModal close={closeSubstrateModal} />}
    </>
  );
}

export { Wallet };
