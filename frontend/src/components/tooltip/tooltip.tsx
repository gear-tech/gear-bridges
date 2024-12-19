import { ReactNode } from 'react';

import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import { Skeleton } from '../layout';

import QuestionSVG from './question.svg?react';
import styles from './tooltip.module.scss';

type Props = {
  value?: ReactNode;
  position?: 'top' | 'bottom-end';
  SVG?: SVGComponent;
  children?: ReactNode;
};

function Tooltip({ value, position = 'top', SVG = QuestionSVG, children }: Props) {
  return (
    <div className={styles.container}>
      <div className={styles.body}>{children || <SVG />}</div>

      <div className={cx(styles.tooltip, styles[position])}>
        {typeof value === 'string' ? <p className={styles.heading}>{value}</p> : value}
      </div>
    </div>
  );
}

function TooltipSkeleton({ disabled }: { disabled?: boolean }) {
  return <Skeleton width="14px" height="14px" borderRadius="50%" className={styles.skeleton} disabled={disabled} />;
}

Tooltip.Skeleton = TooltipSkeleton;

export { Tooltip };
