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
  buttons: {
    value: T;
    text: string;
    description?: string;
    badge?: string;
    SVG: SVGComponent;
    SVGColorType?: 'fill' | 'stroke';
  }[];
  disabled: boolean;
  tooltip: () => JSX.Element;
  onChange: (value: T) => void;
  advanced?: boolean;
  advancedLabel?: string;
  advancedWarning?: string;
};

function Setting<T extends string>({
  value,
  heading,
  tooltip: TooltipContent,
  buttons,
  disabled,
  onChange,
  advanced = false,
  advancedLabel = 'Advanced options',
  advancedWarning,
}: Props<T>) {
  const [isTooltipOpen, setIsTooltipOpen] = useState(false);
  const [isAdvancedOpen, setIsAdvancedOpen] = useState(false);

  const renderButton = ({
    text,
    description,
    badge,
    SVG,
    SVGColorType = 'fill',
    ...button
  }: Props<T>['buttons'][number]) => {
    const isSelected = value === button.value;

    return (
      <button
        key={button.value}
        type="button"
        className={cx(styles.button, isSelected && styles.selected)}
        disabled={disabled}
        aria-pressed={isSelected}
        onClick={() => onChange(button.value)}>
        <SVG className={styles[SVGColorType]} />
        <span className={styles.copy}>
          <span className={styles.labelRow}>
            <span>{text}</span>
            {badge && <span className={styles.badge}>{badge}</span>}
          </span>
          {description && <span className={styles.description}>{description}</span>}
        </span>
      </button>
    );
  };

  const renderButtons = () => {
    if (!advanced) return <div className={styles.buttons}>{buttons.map(renderButton)}</div>;

    const [recommendedButton, advancedButton] = buttons;
    const isAdvancedSelected = value === advancedButton.value;

    return (
      <div className={styles.claimOptions}>
        {renderButton(recommendedButton)}

        <button
          type="button"
          className={styles.advancedToggle}
          disabled={disabled}
          aria-expanded={isAdvancedOpen}
          onClick={() => setIsAdvancedOpen((currentValue) => !currentValue)}>
          <span className={styles.advancedIcon} aria-hidden="true">
            +
          </span>
          <span>{advancedLabel}</span>
          {isAdvancedSelected && <span className={styles.selectedLabel}>{advancedButton.text} selected</span>}
          <span className={cx(styles.chevron, isAdvancedOpen && styles.open)} aria-hidden="true" />
        </button>

        {isAdvancedOpen && (
          <div className={styles.advancedPanel}>
            {renderButton(advancedButton)}
            {advancedWarning && <p className={styles.warning}>{advancedWarning}</p>}
          </div>
        )}
      </div>
    );
  };

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

      <div className={cx(disabled && styles.disabled)}>{renderButtons()}</div>
    </div>
  );
}

export { Setting };
