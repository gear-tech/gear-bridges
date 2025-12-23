import { Tooltip as BaseTooltip } from '@base-ui/react';
import { ComponentProps, ReactElement, ReactNode } from 'react';

import styles from './tooltip.module.scss';

type Props = {
  value: ReactNode;
  children: ReactElement;
  side?: ComponentProps<typeof BaseTooltip.Positioner>['side'];
  isOpen?: boolean;
  onOpenChange?: (value: boolean) => void;
};

function Tooltip({ value, children, side, isOpen, onOpenChange }: Props) {
  if (!value) return children;

  return (
    <BaseTooltip.Provider>
      <BaseTooltip.Root open={isOpen} onOpenChange={onOpenChange}>
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
