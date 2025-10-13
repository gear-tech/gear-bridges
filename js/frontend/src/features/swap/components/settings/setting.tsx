import { Button } from '@gear-js/vara-ui';
import { JSX, useState } from 'react';

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
  const [isTooltipOpen, setIsTooltipOpen] = useState(false);

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

        {/* is it the right way to support tooltips at mobile devices? */}
        {/* feels like tooltip -> button makes more sense, but clicks aren't affecting tooltip visibility then */}
        {/* https://github.com/mui/base-ui/issues/559 */}
        <Button color="transparent" onClick={() => setIsTooltipOpen((prevValue) => !prevValue)}>
          <Tooltip value={<TooltipContent />} isOpen={isTooltipOpen} onOpenChange={setIsTooltipOpen}>
            <OutlineWarningSVG className={styles.tooltip} />
          </Tooltip>
        </Button>
      </h4>

      <div className={cx(styles.buttons, isFirstSelected && styles.active, disabled && styles.disabled)}>
        {renderButtons()}
      </div>
    </div>
  );
}

export { Setting };
