import { HexString } from '@gear-js/api';

import { Skeleton } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { cx, isUndefined } from '@/utils';

import { TOKEN_ID, TokenId, useTokenPrices } from '../../api';

import styles from './token-price.module.scss';

type BaseProps = {
  amount: string | undefined;
  className?: string;
};

type AddressProps = BaseProps & { address: HexString | undefined };
type IdProps = BaseProps & { id: TokenId };
type Props = AddressProps | IdProps;

const TOKEN_ADDRESS_ID = {
  [WRAPPED_VARA_CONTRACT_ADDRESS]: TOKEN_ID.VARA,
  '0x01': TOKEN_ID.ETH,
  '0x02': TOKEN_ID.USDC,
  '0x03': TOKEN_ID.USDT,
} as const;

const round = (value: number) => Number(value.toFixed(3));

function TokenPrice({ amount, className, ...props }: Props) {
  const { data, isLoading } = useTokenPrices();

  const getTokenId = () => {
    if ('address' in props) return props.address ? TOKEN_ADDRESS_ID[props.address] : undefined;

    return props.id;
  };

  const tokenId = getTokenId();
  const price = data && tokenId ? data[tokenId]?.usd : undefined;

  const getPrice = () => {
    if (!price || !amount) return 0;

    const value = price * Number(amount);

    return isNaN(value) ? 0 : round(value);
  };

  return (
    <span className={cx(styles.price, className)}>
      {isUndefined(price) || isUndefined(amount) ? <Skeleton width="2rem" disabled={!isLoading} /> : `$ ${getPrice()}`}
    </span>
  );
}

export { TokenPrice };
