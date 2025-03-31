import { Tooltip } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useVaraFTBalance } from '@/hooks';

import WarningSVG from '../../assets/warning.svg?react';

function LockedBalanceTooltip() {
  const { data: lockedBalance } = useVaraFTBalance(WRAPPED_VARA_CONTRACT_ADDRESS);

  if (!lockedBalance) return;

  return <Tooltip SVG={WarningSVG} value="You have tokens available to unlock" />;
}

export { LockedBalanceTooltip };
