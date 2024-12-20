import { formatBalance } from '@polkadot/util';
import { ComponentProps } from 'react';
import { formatUnits } from 'viem';

import { cx } from '@/utils';

import { Tooltip } from '../tooltip';

import styles from './fortmatted-balance.module.scss';

type Props = {
  value: bigint;
  decimals: number;
  symbol: string;
  tooltipPosition?: ComponentProps<typeof Tooltip>['position'];
  className?: string;
};

function FormattedBalance({ value, decimals, symbol, tooltipPosition, className }: Props) {
  const formattedValue = formatUnits(value, decimals);
  const compactBalance = formatBalance(value, { decimals, withUnit: symbol, withZero: false });

  return (
    <Tooltip value={`${formattedValue} ${symbol}`} position={tooltipPosition}>
      <span className={cx(styles.balance, className)}>
        {compactBalance === '0' ? `${compactBalance} ${symbol}` : compactBalance}
      </span>
    </Tooltip>
  );
}

export { FormattedBalance };
