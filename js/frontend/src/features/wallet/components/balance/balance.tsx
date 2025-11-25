import { useApi } from '@gear-js/react-hooks';
import { WalletModal } from '@gear-js/wallet-connect';
import { useAppKit } from '@reown/appkit/react';

import EthSVG from '@/assets/eth.svg?react';
import VaraSVG from '@/assets/vara.svg?react';
import { FormattedBalance, Skeleton } from '@/components';
import { useEthAccountBalance, useModal, useVaraAccountBalance, useVaraSymbol } from '@/hooks';
import { PropsWithClassName, SVGComponent } from '@/types';
import { cx, isUndefined } from '@/utils';

import styles from './balance.module.scss';

type Props = PropsWithClassName & {
  symbol: string;
  decimals: number;
  icon: SVGComponent;
  useBalance: () => { data: bigint | undefined };
  onClick: () => void;
};

function BalanceComponent({ symbol, decimals, icon: Icon, className, useBalance, onClick }: Props) {
  const { data } = useBalance();

  if (isUndefined(data)) return <Skeleton width="9rem" />;

  return (
    <button type="button" className={cx(styles.balance, className)} onClick={() => onClick()}>
      <Icon />
      <FormattedBalance value={data} decimals={decimals} symbol={symbol} />
    </button>
  );
}

function VaraBalance(props: PropsWithClassName) {
  const { api } = useApi();
  const symbol = useVaraSymbol();
  const [isOpen, openModal, closeModal] = useModal();

  const decimals = api?.registry.chainDecimals[0];

  if (!symbol || isUndefined(decimals)) return;

  return (
    <>
      <BalanceComponent
        symbol={symbol}
        decimals={decimals}
        icon={VaraSVG}
        useBalance={useVaraAccountBalance}
        onClick={openModal}
        {...props}
      />

      {isOpen && <WalletModal close={closeModal} />}
    </>
  );
}

function EthBalance(props: PropsWithClassName) {
  const { open } = useAppKit();

  return (
    <BalanceComponent
      symbol="ETH"
      decimals={18}
      icon={EthSVG}
      useBalance={useEthAccountBalance}
      onClick={open}
      {...props}
    />
  );
}

const Balance = {
  Vara: VaraBalance,
  Eth: EthBalance,
};

export { Balance };
