import { ReactNode } from 'react';

import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import { Skeleton } from '../layout';

import QuestionSVG from './question.svg?react';
import styles from './tooltip.module.scss';

type BaseProps = {
  text?: string;
  children?: ReactNode;
  position?: 'top' | 'bottom-end';
  SVG?: SVGComponent;
};

type TextProps = BaseProps & { text: string };
type ChildrenProps = BaseProps & { children: ReactNode };
type Props = TextProps | ChildrenProps;

function Tooltip({ text, children, position = 'top', SVG = QuestionSVG }: Props) {
  return (
    <div className={styles.container}>
      <SVG />

      <div className={cx(styles.tooltip, styles[position])}>
        {text ? <p className={styles.heading}>{text}</p> : children}
      </div>
    </div>
  );
}

function TooltipSkeleton({ disabled }: { disabled?: boolean }) {
  return <Skeleton width="14px" height="14px" borderRadius="50%" className={styles.skeleton} disabled={disabled} />;
}

Tooltip.Skeleton = TooltipSkeleton;

export { Tooltip };
