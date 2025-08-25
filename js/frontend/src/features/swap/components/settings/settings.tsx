import ClockSVG from '@/assets/clock.svg?react';
import { Tooltip } from '@/components';
import { cx } from '@/utils';

import CircleCheckSVG from '../../assets/circle-check.svg?react';
import HandSVG from '../../assets/hand.svg?react';
import LightningSVG from '../../assets/lightning.svg?react';
import OutlineWarningSVG from '../../assets/outline-warning.svg?react';
import { CLAIM_TYPE, PRIORITY } from '../../consts';

import styles from './settings.module.scss';
import { TooltipContent } from './tooltip-content';

type Props = {
  priority: (typeof PRIORITY)[keyof typeof PRIORITY];
  claimType: (typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE];
  onPriorityChange: (priority: (typeof PRIORITY)[keyof typeof PRIORITY]) => void;
  onClaimTypeChange: (claimType: (typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE]) => void;
};

function Settings({ priority, claimType, onPriorityChange, onClaimTypeChange }: Props) {
  return (
    <div className={styles.settings}>
      <h3 className={styles.heading}>Transfer Settings</h3>

      <div className={styles.body}>
        <div>
          <h4 className={styles.settingHeading}>
            Transfer Speed
            <Tooltip value={<TooltipContent.Priority />}>
              <OutlineWarningSVG className={styles.tooltip} />
            </Tooltip>
          </h4>

          <div className={cx(styles.buttons, priority === PRIORITY.DEFAULT && styles.active)}>
            <button
              type="button"
              className={styles.button}
              onClick={() => onPriorityChange(PRIORITY.DEFAULT)}
              disabled={priority === PRIORITY.DEFAULT}>
              <ClockSVG />
              <span>Common</span>
            </button>

            <button
              type="button"
              className={styles.button}
              onClick={() => onPriorityChange(PRIORITY.HIGH)}
              disabled={priority === PRIORITY.HIGH}>
              <LightningSVG />
              <span>Fast</span>
            </button>
          </div>
        </div>

        <div>
          <h4 className={styles.settingHeading}>
            Claim Type
            <Tooltip value={<TooltipContent.ClaimType />}>
              <OutlineWarningSVG className={styles.tooltip} />
            </Tooltip>
          </h4>

          <div className={cx(styles.buttons, claimType === CLAIM_TYPE.MANUAL && styles.active)}>
            <button
              type="button"
              className={styles.button}
              onClick={() => onClaimTypeChange(CLAIM_TYPE.MANUAL)}
              disabled={claimType === CLAIM_TYPE.MANUAL}>
              <HandSVG className={styles.handIcon} />
              <span>Manual</span>
            </button>

            <button
              type="button"
              className={styles.button}
              onClick={() => onClaimTypeChange(CLAIM_TYPE.AUTO)}
              disabled={claimType === CLAIM_TYPE.AUTO}>
              <CircleCheckSVG />
              <span>Automatic</span>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

export { Settings };
