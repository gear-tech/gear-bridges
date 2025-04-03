import { FormattedBalance, Skeleton } from '@/components';
import { isUndefined } from '@/utils';

import styles from './balance.module.scss';

type Props = {
  value: bigint | undefined;
  decimals: number | undefined;
  symbol: string | undefined;
  isLoading?: boolean;
  heading?: string;
  onMaxButtonClick?: () => void;
};

function Balance({ heading = 'Balance', value, decimals, symbol, isLoading, onMaxButtonClick }: Props) {
  return (
    <div className={styles.balance}>
      <span className={styles.heading}>{heading}:</span>

      {isLoading && <Skeleton width="3rem" />}
      {!isLoading && isUndefined(value) && <Skeleton width="3rem" disabled />}

      {!isUndefined(value) && !isUndefined(decimals) && symbol && (
        <FormattedBalance value={value} decimals={decimals} symbol={symbol} />
      )}

      {Boolean(onMaxButtonClick) && (
        <button type="button" onClick={onMaxButtonClick} disabled={!value || isLoading} className={styles.button}>
          Max
        </button>
      )}
    </div>
  );
}

export { Balance };
