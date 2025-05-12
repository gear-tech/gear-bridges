import { formatUnits } from 'viem';

import { TruncatedText } from '../layout';
import { Tooltip } from '../tooltip';

type Props = {
  value: bigint;
  decimals: number;
  symbol: string;
  className?: string;
};

const FORMATTER = new Intl.NumberFormat('en', {
  notation: 'compact',
  maximumFractionDigits: 4,
});

function FormattedBalance({ value, decimals, symbol, className }: Props) {
  const formattedValue = formatUnits(value, decimals);
  const compactValue = FORMATTER.format(Number(formattedValue));

  const getText = (_value: string) => `${_value} ${symbol}`;

  return (
    <Tooltip value={getText(formattedValue)}>
      <TruncatedText value={getText(compactValue)} className={className} />
    </Tooltip>
  );
}

export { FormattedBalance };
