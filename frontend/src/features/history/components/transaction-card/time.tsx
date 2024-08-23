import { cx } from '@/utils';

import ClockSVG from '../../assets/clock.svg?react';
import { Teleport } from '../../types';

import styles from './transaction-card.module.scss';

type Props = Pick<Teleport, 'timestamp'> & {
  isCompact?: boolean;
};

function Time({ timestamp, isCompact }: Props) {
  const date = new Date(timestamp).toLocaleString();

  return (
    <p className={cx(styles.date, isCompact && styles.compact)}>
      <ClockSVG /> {date}
    </p>
  );
}

export { Time };
