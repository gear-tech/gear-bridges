import { formatBalance } from '@polkadot/util';
import { formatUnits } from 'viem';

import { TruncatedText } from '../layout';
import { Tooltip } from '../tooltip';

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
    <Tooltip value={`${formattedValue} ${symbol}`}>
      <TruncatedText
        value={compactBalance === '0' ? `${compactBalance} ${symbol}` : compactBalance}
        className={className}
      />
    </Tooltip>
  );
}

export { FormattedBalance };
