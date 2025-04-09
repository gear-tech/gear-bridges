import ClockSVG from '@/assets/clock.svg?react';
import { TOKEN_ID, TokenPrice } from '@/features/token-price';
import { cx } from '@/utils';

import { Skeleton } from '../layout';

import styles from './fee-and-time-footer.module.scss';
import GasSVG from './gas.svg?react';

type Props = {
  fee: string | undefined;
  symbol: 'VARA' | 'ETH';
  className?: string;
};

function FeeAndTimeFooter({ fee, symbol, className }: Props) {
  return (
    <footer className={cx(styles.footer, className)}>
      <p className={styles.prop}>
        <span className={styles.key}>
          <GasSVG /> Expected Fee:
        </span>

        <span className={styles.value}>
          {fee ? `${fee} ${symbol}` : <Skeleton width="3.5rem" />}
          <TokenPrice id={symbol === 'VARA' ? TOKEN_ID.VARA : TOKEN_ID.ETH} amount={fee} />
        </span>
      </p>

      <p className={styles.prop}>
        <span className={styles.key}>
          <ClockSVG /> Bridge Time:
        </span>

        <span className={styles.value}>~30 mins</span>
      </p>
    </footer>
  );
}

export { FeeAndTimeFooter };
