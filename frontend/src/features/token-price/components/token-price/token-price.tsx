import { HexString } from '@gear-js/api';

import { Skeleton } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import {
  WRAPPED_ETH_CONTRACT_ADDRESS,
  WRAPPED_USDC_CONTRACT_ADDRESS,
  WRAPPED_USDT_CONTRACT_ADDRESS,
  ETH_WRAPPED_ETH_CONTRACT_ADDRESS,
  ETH_WRAPPED_VARA_CONTRACT_ADDRESS,
  USDC_CONTRACT_ADDRESS,
  USDT_CONTRACT_ADDRESS,
} from '@/consts/env';
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
  [WRAPPED_ETH_CONTRACT_ADDRESS]: TOKEN_ID.ETH,
  [WRAPPED_USDC_CONTRACT_ADDRESS]: TOKEN_ID.USDC,
  [WRAPPED_USDT_CONTRACT_ADDRESS]: TOKEN_ID.USDT,

  [ETH_WRAPPED_ETH_CONTRACT_ADDRESS]: TOKEN_ID.ETH,
  [ETH_WRAPPED_VARA_CONTRACT_ADDRESS]: TOKEN_ID.VARA,
  [USDC_CONTRACT_ADDRESS]: TOKEN_ID.USDC,
  [USDT_CONTRACT_ADDRESS]: TOKEN_ID.USDT,
} as const;

const FORMATTER = new Intl.NumberFormat('en', {
  style: 'currency',
  currency: 'USD',
});

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

    return FORMATTER.format(isNaN(value) ? 0 : value);
  };

  return (
    <span className={cx(styles.price, className)}>
      {isUndefined(price) || isUndefined(amount) ? <Skeleton width="2rem" disabled={!isLoading} /> : getPrice()}
    </span>
  );
}

export { TokenPrice };
