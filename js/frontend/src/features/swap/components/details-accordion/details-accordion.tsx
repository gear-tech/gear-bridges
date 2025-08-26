import { ComponentProps } from 'react';

import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import { FeeAndTimeFooter } from '../fee-and-time-footer';

import styles from './details-accordion.module.scss';

type Props = ComponentProps<typeof FeeAndTimeFooter> & {
  isOpen: boolean;
  onToggle: () => void;
};

function DetailsAccordion({ isOpen, onToggle, ...props }: Props) {
  return (
    <div className={cx(styles.details, isOpen && styles.open)}>
      <button type="button" className={styles.button} onClick={onToggle}>
        <span>Details</span>
        <ArrowSVG className={styles.icon} />
      </button>

      <FeeAndTimeFooter {...props} className={styles.body} />
    </div>
  );
}

export { DetailsAccordion };
