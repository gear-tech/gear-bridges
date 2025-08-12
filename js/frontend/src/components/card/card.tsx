import { ComponentPropsWithoutRef, ElementType, PropsWithChildren } from 'react';

import { cx } from '@/utils';

import styles from './card.module.scss';

type Props<T extends ElementType> = PropsWithChildren &
  ComponentPropsWithoutRef<T> & {
    as?: T;
    className?: string;
  };

function Card<T extends ElementType = 'div'>({ as, className, ...props }: Props<T>) {
  const Element = as || 'div';

  return <Element className={cx(styles.card, className)} {...props} />;
}

export { Card };
