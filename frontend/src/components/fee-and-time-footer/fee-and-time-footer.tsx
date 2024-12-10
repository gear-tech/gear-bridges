import ClockSVG from '@/assets/clock.svg?react';

import { Skeleton } from '../layout';

import styles from './fee-and-time-footer.module.scss';
import GasSVG from './gas.svg?react';

type Props = {
  fee: string | undefined;
  symbol: string;
};

function FeeAndTimeFooter({ fee, symbol }: Props) {
  return (
    <footer className={styles.footer}>
      <p className={styles.prop}>
        <span>Fee:</span>

        <span className={styles.value}>
          <GasSVG />
          {fee ? `${fee} ${symbol}` : <Skeleton width="3.5rem" />}
        </span>
      </p>

      <p className={styles.prop}>
        <span>Bridge Time:</span>

        <span className={styles.value}>
          <ClockSVG />
          ~30 mins
        </span>
      </p>
    </footer>
  );
}

export { FeeAndTimeFooter };
