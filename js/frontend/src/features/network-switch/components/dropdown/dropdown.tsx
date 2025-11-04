import { Menu, Separator } from '@base-ui-components/react';

import { NETWORK_TYPE } from '@/context/network-type/consts';
import ActionArrowSVG from '@/features/swap/assets/arrow.svg?react';
import { cx } from '@/utils';

import ArrowSVG from '../../assets/arrow.svg?react';
import SpinnerSVG from '../../assets/spinner.svg?react';
import WorldSVG from '../../assets/world.svg?react';

import styles from './dropdown.module.scss';

type Props = {
  value: string;
  isLoading: boolean;
  onChange: (value: 'mainnet' | 'testnet') => void;
};

function Dropdown({ value, isLoading, onChange }: Props) {
  return (
    <Menu.Root>
      <Menu.Trigger
        className={styles.button}
        disabled={isLoading}
        render={(props, state) => (
          <button {...props}>
            {isLoading ? <SpinnerSVG className={styles.spinnerSvg} /> : <WorldSVG className={styles.networkSvg} />}
            {value === NETWORK_TYPE.MAINNET ? 'Mainnet' : 'Testnet'}
            <ActionArrowSVG className={cx(styles.arrowSvg, isLoading && styles.loading, state.open && styles.open)} />
          </button>
        )}
      />

      <Menu.Portal>
        <Menu.Positioner sideOffset={12}>
          <Menu.Popup className={styles.popup}>
            <Menu.Arrow className={styles.arrow}>
              <ArrowSVG />
            </Menu.Arrow>

            <Menu.RadioGroup value={value} onValueChange={onChange}>
              <Menu.RadioItem className={styles.item} value="mainnet" disabled={isLoading} closeOnClick>
                <span className={styles.itemContent}>
                  <span>Mainnet</span>
                  <Menu.RadioItemIndicator className={styles.indicator} />
                </span>
              </Menu.RadioItem>

              <Separator className={styles.separator} />

              <Menu.RadioItem className={styles.item} value="testnet" disabled={isLoading} closeOnClick>
                <span className={styles.itemContent}>
                  <span>Testnet</span>
                  <Menu.RadioItemIndicator className={styles.indicator} />
                </span>
              </Menu.RadioItem>
            </Menu.RadioGroup>
          </Menu.Popup>
        </Menu.Positioner>
      </Menu.Portal>
    </Menu.Root>
  );
}

export { Dropdown };
