import WarningSVG from '@/assets/warning.svg?react';
import { Tooltip } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { useVaraFTBalance } from '@/hooks';

function LockedBalanceTooltip() {
  const { data: lockedBalance } = useVaraFTBalance(WRAPPED_VARA_CONTRACT_ADDRESS);

  if (!lockedBalance) return;

  return (
    <Tooltip value="You have tokens available to unlock">
      <WarningSVG />
    </Tooltip>
  );
}

export { LockedBalanceTooltip };
