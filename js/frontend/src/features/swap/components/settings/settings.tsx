import CircleCheckSVG from '../../assets/circle-check.svg?react';
import HandSVG from '../../assets/hand.svg?react';
import { CLAIM_TYPE } from '../../consts';
import { FeeAndTimeFooter } from '../fee-and-time-footer';

import { Setting } from './setting';
import styles from './settings.module.scss';
import { TooltipContent } from './tooltip-content';

const CLAIM_TYPE_BUTTONS = [
  { value: CLAIM_TYPE.AUTO, text: 'Automatic', SVG: CircleCheckSVG },
  { value: CLAIM_TYPE.MANUAL, text: 'Manual', SVG: HandSVG, SVGColorType: 'stroke' as const },
];

type ClaimType = (typeof CLAIM_TYPE)[keyof typeof CLAIM_TYPE];

type Props = {
  isVaraNetwork: boolean;
  claimType: ClaimType;
  disabled: boolean;
  fee: bigint | undefined;
  time: string;
  isFeeLoading: boolean;
  onClaimTypeChange: (claimType: ClaimType) => void;
};

function Settings({ isVaraNetwork, claimType, disabled, fee, time, isFeeLoading, onClaimTypeChange }: Props) {
  return (
    <div className={styles.settings}>
      <h3 className={styles.heading}>Transfer Settings</h3>

      <div className={styles.body}>
        <Setting
          heading="Claim Type"
          tooltip={TooltipContent.ClaimType}
          buttons={CLAIM_TYPE_BUTTONS}
          value={claimType}
          onChange={onClaimTypeChange}
          disabled={disabled}
        />
      </div>

      <FeeAndTimeFooter isVaraNetwork={isVaraNetwork} feeValue={fee} time={time} isLoading={isFeeLoading} />
    </div>
  );
}

export { Settings };
