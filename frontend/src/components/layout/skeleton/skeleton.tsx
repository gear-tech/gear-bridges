import { ReactNode } from 'react';

import { cx } from '@/utils';

import styles from './skeleton.module.scss';

type Props = {
  width?: string;
  height?: string;
  borderRadius?: string;
  children?: ReactNode;
  disabled?: boolean;
};

function Skeleton({ width, height, borderRadius, children, disabled }: Props) {
  return (
    <span className={cx(styles.skeleton, !disabled && styles.loading)} style={{ width, height, borderRadius }}>
      {children}
    </span>
  );
}

export { Skeleton };
