import { Button } from '@gear-js/vara-ui';

import { Skeleton } from '@/components';

import styles from './balance.module.scss';

type Props = {
  value: string | undefined;
  unit: string | undefined;
  isLoading?: boolean;
  heading?: string;
  onMaxButtonClick?: () => void;
};

function Balance({ heading = 'Balance', value, unit, isLoading, onMaxButtonClick }: Props) {
  return (
    <div className={styles.balance}>
      <header className={styles.header}>
        <span className={styles.heading}>{heading}:</span>

        {Boolean(onMaxButtonClick) && (
          <Button text="Max" color="transparent" onClick={onMaxButtonClick} disabled={!value} isLoading={isLoading} />
        )}
      </header>

      <p className={styles.value}>
        {isLoading && <Skeleton />}
        {!isLoading && !value && <Skeleton disabled />}
        {value && unit && `${value} ${unit}`}
      </p>
    </div>
  );
}

export { Balance };
