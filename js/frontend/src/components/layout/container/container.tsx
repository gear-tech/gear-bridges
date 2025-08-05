import { ReactNode } from 'react';

import { cx } from '@/utils';

import styles from './container.module.scss';

type Props = {
  children: ReactNode;
  maxWidth?: `${string}px`;
  className?: string;
};

function Container({ children, maxWidth, className }: Props) {
  return (
    <div className={cx(styles.container, className)} style={{ maxWidth }}>
      {children}
    </div>
  );
}

export { Container };
