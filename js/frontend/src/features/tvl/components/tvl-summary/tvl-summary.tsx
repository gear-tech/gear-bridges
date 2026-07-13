import { Skeleton } from '@/components';

import { formatTvl } from '../../utils/format-tvl';

import styles from './tvl-summary.module.scss';

type Props = {
  value?: number;
  isLoading: boolean;
};

function TvlSummary({ value, isLoading }: Props) {
  return (
    <div className={styles.summary}>
      <p className={styles.label}>Total Value Locked</p>

      {isLoading ? (
        <Skeleton width="240px" height="48px" borderRadius="8px" />
      ) : (
        <p className={styles.value}>{formatTvl(value ?? 0)}</p>
      )}
    </div>
  );
}

export { TvlSummary };
