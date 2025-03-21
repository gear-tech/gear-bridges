import { ComponentProps, useState } from 'react';

import { FeeAndTimeFooter } from '@/components';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';

import styles from './details-accordion.module.scss';

type Props = ComponentProps<typeof FeeAndTimeFooter>;

function DetailsAccordion(props: Props) {
  const [isOpen, setIsOpen] = useState(false);

  return (
    <div className={cx(styles.details, isOpen && styles.open)}>
      <button type="button" className={styles.button} onClick={() => setIsOpen((prevValue) => !prevValue)}>
        <span>Details</span>
        <ArrowSVG className={styles.icon} />
      </button>

      <FeeAndTimeFooter {...props} className={styles.body} />
    </div>
  );
}

export { DetailsAccordion };
