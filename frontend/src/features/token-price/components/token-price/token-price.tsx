import { HexString } from '@gear-js/api';

import { Skeleton } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { isUndefined } from '@/utils';

import { TOKEN_ID, useTokenPrices } from '../../api';

import styles from './token-price.module.scss';

type Props = {
  address: HexString | undefined;
  amount: string;
};

const TOKEN_ADDRESS_ID = {
  [WRAPPED_VARA_CONTRACT_ADDRESS]: TOKEN_ID.VARA,
  '0x01': TOKEN_ID.ETH,
  '0x02': TOKEN_ID.USDC,
  '0x03': TOKEN_ID.USDT,
} as const;

const round = (value: number) => Number(value.toFixed(3));

function TokenPrice({ address, amount }: Props) {
  const amountNum = Number(amount);

  const { data, isLoading } = useTokenPrices();
  const tokenId = address ? TOKEN_ADDRESS_ID[address] : undefined;
  const price = tokenId && data ? data[tokenId]?.usd : undefined;

  return (
    <p className={styles.price}>
      {isUndefined(price) ? <Skeleton width="2rem" disabled={!isLoading} /> : `$ ${round(price * amountNum)}`}
    </p>
  );
}

export { TokenPrice };
