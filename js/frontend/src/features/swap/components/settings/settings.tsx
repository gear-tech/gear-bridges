import { useState } from 'react';
import { Link } from 'react-router-dom';

import ClockSVG from '@/assets/clock.svg?react';
import { Tooltip } from '@/components';
import { ROUTE } from '@/consts';
import { cx } from '@/utils';

import CircleCheckSVG from '../../assets/circle-check.svg?react';
import HandSVG from '../../assets/hand.svg?react';
import LightningSVG from '../../assets/lightning.svg?react';
import OutlineWarningSVG from '../../assets/outline-warning.svg?react';

import styles from './settings.module.scss';

const PRIORITY = {
  COMMON: 'common',
  FAST: 'fast',
} as const;

const CLAIM_TYPE = {
  MANUAL: 'manual',
  AUTO: 'auto',
} as const;

function PriorityTooltipContent() {
  return (
    <>
      <p>
        <span className={styles.bold}>Transfer Speed</span> defines how quickly your transfer will be processed:
      </p>

      <ul className={styles.list}>
        <li>
          <span className={styles.bold}>Common</span> - standard speed (~1 hour) with a lower fee.
        </li>

        <li>
          <span className={styles.bold}>Fast</span> - accelerated processing (~20 minutes) with a higher fee.
        </li>
      </ul>
    </>
  );
}

function ClaimTypeTooltipContent() {
  return (
    <>
      <p>
        <span className={styles.bold}>Claim Type</span> determines how you receive your tokens:
      </p>

      <ul className={styles.list}>
        <li>
          <span className={styles.bold}>Manual</span> - after the transfer is completed, you need to manually claim your
          tokens from the{' '}
          <Link to={ROUTE.TOKEN_TRACKER} className={styles.link}>
            My Tokens
          </Link>{' '}
          page.
        </li>

        <li>
          <span className={styles.bold}>Automatic</span> - tokens are delivered to your wallet automatically, for an
          additional fee.
        </li>
      </ul>
    </>
  );
}

function Settings() {
  const [priority, setPriority] = useState<(typeof PRIORITY)[keyof typeof PRIORITY]>(PRIORITY.COMMON);
  const [claimType, setClaimType] = useState<(typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE]>(CLAIM_TYPE.MANUAL);

  return (
    <div className={styles.settings}>
      <h3 className={styles.heading}>Transfer Settings</h3>

      <div className={styles.body}>
        <div>
          <h4 className={styles.settingHeading}>
            Transfer Speed
            <Tooltip value={<PriorityTooltipContent />}>
              <OutlineWarningSVG className={styles.tooltip} />
            </Tooltip>
          </h4>

          <div className={cx(styles.buttons, priority === PRIORITY.COMMON && styles.active)}>
            <button
              type="button"
              className={styles.button}
              onClick={() => setPriority(PRIORITY.COMMON)}
              disabled={priority === PRIORITY.COMMON}>
              <ClockSVG />
              <span>Common</span>
            </button>

            <button
              type="button"
              className={styles.button}
              onClick={() => setPriority(PRIORITY.FAST)}
              disabled={priority === PRIORITY.FAST}>
              <LightningSVG />
              <span>Fast</span>
            </button>
          </div>
        </div>

        <div>
          <h4 className={styles.settingHeading}>
            Claim Type
            <Tooltip value={<ClaimTypeTooltipContent />}>
              <OutlineWarningSVG className={styles.tooltip} />
            </Tooltip>
          </h4>

          <div className={cx(styles.buttons, claimType === CLAIM_TYPE.MANUAL && styles.active)}>
            <button
              type="button"
              className={styles.button}
              onClick={() => setClaimType(CLAIM_TYPE.MANUAL)}
              disabled={claimType === CLAIM_TYPE.MANUAL}>
              <HandSVG className={styles.handIcon} />
              <span>Manual</span>
            </button>

            <button
              type="button"
              className={styles.button}
              onClick={() => setClaimType(CLAIM_TYPE.AUTO)}
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
