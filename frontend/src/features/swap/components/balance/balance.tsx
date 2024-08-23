import { Button } from '@gear-js/vara-ui';

import { Skeleton } from '@/components';
import { SVGComponent } from '@/types';

import styles from './balance.module.scss';

type Props = {
  value: string | undefined;
  unit: string;
  isLoading: boolean;
  heading?: string;
  SVG?: SVGComponent;
  onMaxButtonClick?: () => void;
};

function Balance({ heading = 'Balance', value, unit, isLoading, SVG, onMaxButtonClick }: Props) {
  return (
    <div className={styles.container}>
      {SVG && <SVG />}

      <div className={styles.balance}>
        <header className={styles.header}>
          <span className={styles.heading}>{heading}:</span>

          {Boolean(onMaxButtonClick) && (
            <Button
              text="Use Max"
              color="transparent"
              onClick={onMaxButtonClick}
              disabled={!value}
              isLoading={isLoading}
            />
          )}
        </header>

        <p>
          {isLoading && <Skeleton />}
          {!isLoading && !value && <Skeleton disabled />}
          {value && `${value} ${unit}`}
        </p>
      </div>
    </div>
  );
}

export { Balance };
