import { ComponentProps } from 'react';

import { cx } from '@/utils';

import { Balance } from '../balance';

import styles from './balance-card.module.scss';

type Props = ComponentProps<typeof Balance> & {
  locked?: boolean;
};

function BalanceCard({ locked, ...props }: Props) {
  return (
    <div className={cx(styles.card, locked && styles.locked)}>
      <Balance {...props} />
    </div>
  );
}

export { BalanceCard };
