import { formatUnits } from 'viem';

import { cx } from '@/utils';

import { Tooltip } from '../tooltip';

import styles from './fortmatted-balance.module.scss';

// simplest solution without rounding for now
const withPrecision = (value: string) => {
  const DIGITS_COUNT = 2;
  const decimalIndex = value.indexOf('.');

  if (decimalIndex === -1) return value;

  return value.slice(0, decimalIndex + DIGITS_COUNT + 1);
};

type Props = {
  value: bigint;
  decimals: number;
  symbol: string;
  className?: string;
};

function FormattedBalance({ value, decimals, symbol, className }: Props) {
  const formattedValue = formatUnits(value, decimals);

  return (
    <span className={cx(styles.balance, className)}>
      {withPrecision(formattedValue)} {symbol}
      <Tooltip text={formattedValue} />
    </span>
  );
}

export { FormattedBalance };
