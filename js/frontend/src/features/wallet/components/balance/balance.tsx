import { useApi } from '@gear-js/react-hooks';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { FormattedBalance, Skeleton } from '@/components';
import { useEthAccountBalance, useModal, useVaraAccountBalance, useVaraSymbol } from '@/hooks';
import { SVGComponent } from '@/types';
import { isUndefined } from '@/utils';

import styles from './balance.module.scss';

type Props = {
  symbol: string | undefined;
  decimals: number | undefined;
  icon: SVGComponent;
  useBalance: () => { data: bigint | undefined };
  onClick: () => void;
};

function BalanceComponent({ symbol, decimals, icon: Icon, useBalance, onClick }: Props) {
  const { data } = useBalance();

  if (isUndefined(data) || isUndefined(symbol) || isUndefined(decimals))
    return <Skeleton height="2rem" className={styles.skeleton} />;

  return (
    <button type="button" className={styles.balance} onClick={() => onClick()}>
      <Icon />
      <FormattedBalance value={data} decimals={decimals} symbol={symbol} />
    </button>
  );
}

function VaraBalance() {
  const { api } = useApi();
  const symbol = useVaraSymbol();
  const [isOpen, openModal, closeModal] = useModal();

  const decimals = api?.registry.chainDecimals[0];

  return (
    <>
      <BalanceComponent
        symbol={symbol}
        decimals={decimals}
        icon={VaraSVG}
        useBalance={useVaraAccountBalance}
        onClick={openModal}
      />

      {isOpen && <WalletModal close={closeModal} />}
    </>
  );
}

function EthBalance() {
  const { open } = useAppKit();

  return <BalanceComponent symbol="ETH" decimals={18} icon={EthSVG} useBalance={useEthAccountBalance} onClick={open} />;
}

const Balance = {
  Vara: VaraBalance,
  Eth: EthBalance,
};

export { Balance };
