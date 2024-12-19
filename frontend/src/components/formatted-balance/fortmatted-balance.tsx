import { formatBalance } from '@polkadot/util';
import { formatUnits } from 'viem';

import { cx } from '@/utils';

import { Tooltip } from '../tooltip';

import styles from './fortmatted-balance.module.scss';

type Props = {
  value: bigint;
  decimals: number;
  symbol: string;
  className?: string;
};

function FormattedBalance({ value, decimals, symbol, className }: Props) {
  const formattedValue = formatUnits(value, decimals);
  const compactBalance = formatBalance(value, { decimals, withUnit: symbol, withZero: false });

  return (
    <span className={cx(styles.balance, className)}>
      {compactBalance}
      <Tooltip text={`${formattedValue} ${symbol}`} />
    </span>
  );
}

export { FormattedBalance };
