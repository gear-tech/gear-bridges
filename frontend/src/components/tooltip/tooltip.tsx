import { Tooltip as BaseTooltip } from '@base-ui-components/react';
import { ReactElement, ReactNode } from 'react';

import styles from './tooltip.module.scss';

type Props = {
  value: ReactNode;
  children: ReactElement;
};

function Tooltip({ value, children }: Props) {
  return (
    <BaseTooltip.Provider>
      <BaseTooltip.Root>
        <BaseTooltip.Trigger render={children as ReactElement<Record<string, unknown>>} />

        <BaseTooltip.Portal>
          <BaseTooltip.Positioner sideOffset={8}>
            <BaseTooltip.Popup className={styles.popup}>{value}</BaseTooltip.Popup>
          </BaseTooltip.Positioner>
        </BaseTooltip.Portal>
      </BaseTooltip.Root>
    </BaseTooltip.Provider>
  );
}

export { Tooltip };
