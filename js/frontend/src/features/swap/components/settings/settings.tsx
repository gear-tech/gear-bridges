import { ComponentProps } from 'react';

import ClockSVG from '@/assets/clock.svg?react';

import CircleCheckSVG from '../../assets/circle-check.svg?react';
import HandSVG from '../../assets/hand.svg?react';
import LightningSVG from '../../assets/lightning.svg?react';
import { CLAIM_TYPE, PRIORITY } from '../../consts';
import { FeeAndTimeFooter } from '../fee-and-time-footer';

import { Setting } from './setting';
import styles from './settings.module.scss';
import { TooltipContent } from './tooltip-content';

const PRIORITY_BUTTONS = [
  { value: PRIORITY.DEFAULT, text: 'Common', SVG: ClockSVG },
  { value: PRIORITY.HIGH, text: 'Fast', SVG: LightningSVG },
];

const CLAIM_TYPE_BUTTONS = [
  { value: CLAIM_TYPE.MANUAL, text: 'Manual', SVG: HandSVG, SVGColorType: 'stroke' as const },
  { value: CLAIM_TYPE.AUTO, text: 'Automatic', SVG: CircleCheckSVG },
];

type Priority = (typeof PRIORITY)[keyof typeof PRIORITY];
type ClaimType = (typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE];

type Props = ComponentProps<typeof FeeAndTimeFooter> & {
  priority: Priority;
  claimType: ClaimType;
  onPriorityChange: (priority: Priority) => void;
  onClaimTypeChange: (claimType: ClaimType) => void;
};

function Settings({ isVaraNetwork, priority, claimType, onPriorityChange, onClaimTypeChange, ...props }: Props) {
  return (
    <div className={styles.settings}>
      <h3 className={styles.heading}>Transfer Settings</h3>

      <div className={styles.body}>
        {isVaraNetwork && (
          <Setting
            value={priority}
            onChange={onPriorityChange}
            heading="Transfer Speed"
            tooltip={TooltipContent.Priority}
            buttons={PRIORITY_BUTTONS}
          />
        )}

        <Setting
          value={claimType}
          onChange={onClaimTypeChange}
          heading="Claim Type"
          tooltip={TooltipContent.ClaimType}
          buttons={CLAIM_TYPE_BUTTONS}
        />
      </div>

      <FeeAndTimeFooter isVaraNetwork={isVaraNetwork} {...props} />
    </div>
  );
}

export { Settings };
