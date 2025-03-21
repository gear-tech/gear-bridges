import { ComponentProps } from 'react';

import { Skeleton } from '@/components';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';

import styles from './token-select.module.scss';

type Props = ComponentProps<'select'> & {
  options: { value: string; label: string }[];
  isLoading: boolean;
};

function TokenSelect({ options, className, isLoading, ...props }: Props) {
  const renderOptions = () =>
    options.map(({ value, label }) => (
      <option key={value} value={value}>
        {label}
      </option>
    ));

  if (isLoading) return <Skeleton width="6rem" height="24px" />;

  return (
    <div className={cx(styles.container, className)}>
      <select className={styles.select} {...props}>
        {renderOptions()}
      </select>

      <ArrowSVG className={styles.icon} />
    </div>
  );
}

export { TokenSelect };
