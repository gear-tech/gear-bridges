import { ReactNode } from 'react';

import { cx } from '@/utils';

import styles from './card.module.scss';

type Props = {
  children: ReactNode;
  className?: string;
};

function Card({ children, className }: Props) {
  return <div className={cx(styles.card, className)}>{children}</div>;
}

export { Card };
