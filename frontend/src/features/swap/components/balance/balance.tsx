import { Button } from '@gear-js/vara-ui';

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
      <header className={styles.header}>
        <span className={styles.heading}>{heading}:</span>

        {Boolean(onMaxButtonClick) && (
          <Button text="Max" color="transparent" onClick={onMaxButtonClick} disabled={!value} isLoading={isLoading} />
        )}
      </header>

      <div className={styles.value}>
        {isLoading && <Skeleton />}
        {!isLoading && isUndefined(value) && <Skeleton disabled />}

        {!isUndefined(value) && !isUndefined(decimals) && symbol && (
          <FormattedBalance value={value} decimals={decimals} symbol={symbol} />
        )}
      </div>
    </div>
  );
}

export { Balance };
