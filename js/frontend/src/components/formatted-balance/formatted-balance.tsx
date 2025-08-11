import { formatUnits } from 'viem';

import { cx } from '@/utils';

import { TruncatedText } from '../layout';
import { Tooltip } from '../tooltip';

import styles from './formatted-balance.module.scss';

type Props = {
  value: bigint;
  decimals: number;
  symbol: string;
  truncated?: boolean;
  className?: string;
};

const FORMATTER = new Intl.NumberFormat('en', {
  notation: 'compact',
  maximumFractionDigits: 4,
});

function FormattedBalance({ value, decimals, symbol, truncated = true, className }: Props) {
  const formattedValue = formatUnits(value, decimals);
  const compactValue = FORMATTER.format(Number(formattedValue));

  const getText = (_value: string) => `${_value} ${symbol}`;

  return (
    <Tooltip value={getText(formattedValue)}>
      {truncated ? (
        <TruncatedText value={getText(compactValue)} className={className} />
      ) : (
        <span className={cx(styles.text, className)}>{getText(formattedValue)}</span>
      )}
    </Tooltip>
  );
}

export { FormattedBalance };
