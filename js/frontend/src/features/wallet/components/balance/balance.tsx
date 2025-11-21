import { useApi } from '@gear-js/react-hooks';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import { FormattedBalance, Skeleton } from '@/components';
import { useEthAccountBalance, useModal, useVaraAccountBalance, useVaraSymbol } from '@/hooks';
import { isUndefined } from '@/utils';

import WalletSVG from '../../assets/wallet.svg?react';

import styles from './balance.module.scss';

type Props = {
  symbol: string;
  decimals: number;
  useBalance: () => { data: bigint | undefined };
  onClick: () => void;
};

function BalanceComponent({ symbol, decimals, useBalance, onClick }: Props) {
  const { data } = useBalance();

  if (isUndefined(data)) return <Skeleton width="9rem" />;

  return (
    <button type="button" className={styles.balance} onClick={onClick}>
      <WalletSVG />
      <FormattedBalance value={data} decimals={decimals} symbol={symbol} />
    </button>
  );
}

function VaraBalance() {
  const { api } = useApi();
  const symbol = useVaraSymbol();
  const [isOpen, openModal, closeModal] = useModal();

  const decimals = api?.registry.chainDecimals[0];

  if (!symbol || isUndefined(decimals)) return;

  return (
    <>
      <BalanceComponent symbol={symbol} decimals={decimals} useBalance={useVaraAccountBalance} onClick={openModal} />

      {isOpen && <WalletModal close={closeModal} />}
    </>
  );
}

function EthBalance() {
  const { open } = useAppKit();

  return <BalanceComponent symbol="ETH" decimals={18} useBalance={useEthAccountBalance} onClick={open} />;
}

const Balance = {
  Vara: VaraBalance,
  Eth: EthBalance,
};

export { Balance };
