import { Tooltip as BaseTooltip } from '@base-ui-components/react';
import { ComponentProps, ReactElement, ReactNode } from 'react';

import styles from './tooltip.module.scss';

type Props = {
  value: ReactNode;
  children: ReactElement;
  side?: ComponentProps<typeof BaseTooltip.Positioner>['side'];
};

function Tooltip({ value, children, side }: Props) {
  return (
    <BaseTooltip.Provider>
      <BaseTooltip.Root>
        <BaseTooltip.Trigger render={children as ReactElement<Record<string, unknown>>} />

        <BaseTooltip.Portal>
          <BaseTooltip.Positioner sideOffset={8} side={side} className={styles.positioner}>
            <BaseTooltip.Popup className={styles.popup}>{value}</BaseTooltip.Popup>
          </BaseTooltip.Positioner>
        </BaseTooltip.Portal>
      </BaseTooltip.Root>
    </BaseTooltip.Provider>
  );
}

export { Tooltip };
