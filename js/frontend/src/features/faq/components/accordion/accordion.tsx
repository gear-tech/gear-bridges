import { PropsWithChildren, ReactNode } from 'react';

import { Card } from '@/components';
import { useModal } from '@/hooks';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';

import styles from './accordion.module.scss';

type Props = PropsWithChildren & {
  heading: ReactNode;
};

function Accordion({ heading, children }: Props) {
  const [isOpen, open, close] = useModal();

  return (
    <Card className={cx(styles.accordion, isOpen && styles.open)}>
      <button className={styles.button} onClick={isOpen ? close : open}>
        <h2>{heading}</h2>
        <ArrowSVG className={styles.icon} />
      </button>

      <div className={styles.body}>{children}</div>
    </Card>
  );
}

export { Accordion };
