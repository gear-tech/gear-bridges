import { Skeleton } from '@/components';

import WalletSVG from '../../assets/wallet.svg?react';
import { UseAccountBalance } from '../../types';

import styles from './account-balance.module.scss';

type Props = ReturnType<UseAccountBalance> & {
  symbol: string;
};

function AccountBalance({ value, formattedValue, isLoading, symbol }: Props) {
  if (isLoading || !formattedValue) return <Skeleton />;

  return (
    <div className={styles.balance}>
      <WalletSVG />
      {`${formattedValue} ${symbol}`}
    </div>
  );
}

export { AccountBalance };
