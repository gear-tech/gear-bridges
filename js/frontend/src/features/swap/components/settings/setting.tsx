import { JSX } from 'react';

import { Tooltip } from '@/components';
import { SVGComponent } from '@/types';
import { cx } from '@/utils';

import OutlineWarningSVG from '../../assets/outline-warning.svg?react';

import styles from './setting.module.scss';

type Props<T extends string> = {
  value: T;
  heading: string;
  buttons: { value: T; text: string; SVG: SVGComponent; SVGColorType?: 'fill' | 'stroke' }[];
  disabled: boolean;
  tooltip: () => JSX.Element;
  onChange: (value: T) => void;
};

function Setting<T extends string>({ value, heading, tooltip: TooltipContent, buttons, disabled, onChange }: Props<T>) {
  const isFirstSelected = value === buttons[0].value;

  const renderButtons = () =>
    buttons.map(({ text, SVG, SVGColorType = 'fill', ...button }) => (
      <button
        key={button.value}
        type="button"
        className={styles.button}
        disabled={value === button.value}
        onClick={() => onChange(button.value)}>
        <SVG className={styles[SVGColorType]} />
        <span>{text}</span>
      </button>
    ));

  return (
    <div>
      <h4 className={styles.heading}>
        {heading}

        <Tooltip value={<TooltipContent />}>
          <OutlineWarningSVG className={styles.tooltip} />
        </Tooltip>
      </h4>

      <div className={cx(styles.buttons, isFirstSelected && styles.active, disabled && styles.disabled)}>
        {renderButtons()}
      </div>
    </div>
  );
}

export { Setting };
