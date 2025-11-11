import { useRef } from 'react';

import { Skeleton } from '@/components';
import { cx, isUndefined } from '@/utils';

import { TOKEN_ID, TokenId, useTokenPrices } from '../../api';

import styles from './token-price.module.scss';

type BaseProps = {
  amount: string | undefined;
  fraction?: number;
  className?: string;
};

type SymbolProps = BaseProps & { symbol: string | undefined };
type IdProps = BaseProps & { id: TokenId };
type Props = SymbolProps | IdProps;

function TokenPrice({ amount, className, fraction = 2, ...props }: Props) {
  const { data, isLoading } = useTokenPrices();

  const formatter = useRef(
    new Intl.NumberFormat('en', { style: 'currency', currency: 'USD', maximumFractionDigits: fraction }),
  );

  const getTokenId = () => {
    if ('symbol' in props) {
      const lowerCaseSymbol = props.symbol?.toLowerCase();

      if (lowerCaseSymbol?.includes('vara')) return TOKEN_ID.VARA;
      if (lowerCaseSymbol?.includes('eth')) return TOKEN_ID.ETH;
      if (lowerCaseSymbol?.includes('usdc')) return TOKEN_ID.USDC;
      if (lowerCaseSymbol?.includes('usdt')) return TOKEN_ID.USDT;
      if (lowerCaseSymbol?.includes('btc')) return TOKEN_ID.BTC;

      return;
    }

    return props.id;
  };

  const tokenId = getTokenId();
  const price = data && tokenId ? data[tokenId]?.usd : undefined;

  const getPrice = () => {
    if (!price || !amount) return 0;

    const value = price * Number(amount);

    return formatter.current.format(isNaN(value) ? 0 : value);
  };

  return (
    <span className={cx(styles.price, className)}>
      {isUndefined(price) || isUndefined(amount) ? <Skeleton width="2rem" disabled={!isLoading} /> : getPrice()}
    </span>
  );
}

export { TokenPrice };
