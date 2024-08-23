import { Card } from '@/components';
import { useModal } from '@/hooks';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';

import styles from './accordion.module.scss';

type Props = {
  heading: string;
  text: string;
};

function Accordion({ heading, text }: Props) {
  const [isOpen, open, close] = useModal();

  return (
    <Card className={cx(styles.accordion, isOpen && styles.open)}>
      <button className={styles.button} onClick={isOpen ? close : open}>
        <h2 className={styles.heading}>{heading}</h2>

        <ArrowSVG />
      </button>

      {isOpen && <p className={styles.text}>{text}</p>}
    </Card>
  );
}

export { Accordion };
