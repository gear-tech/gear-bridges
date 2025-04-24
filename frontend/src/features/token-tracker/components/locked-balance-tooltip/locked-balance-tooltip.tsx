import WarningSVG from '@/assets/warning.svg?react';
import { Tooltip } from '@/components';
import { WRAPPED_VARA_CONTRACT_ADDRESS } from '@/consts';
import { ETH_WRAPPED_ETH_CONTRACT_ADDRESS } from '@/consts/env';
import { useEthFTBalance, useVaraFTBalance } from '@/hooks';

function LockedBalanceTooltip() {
  const { data: lockedVaraBalance } = useVaraFTBalance(WRAPPED_VARA_CONTRACT_ADDRESS);
  const { data: lockedEthBalance } = useEthFTBalance(ETH_WRAPPED_ETH_CONTRACT_ADDRESS);
  const lockedBalance = lockedVaraBalance || lockedEthBalance;

  if (!lockedBalance) return;

  return (
    <Tooltip value="You have tokens available to unlock">
      <WarningSVG />
    </Tooltip>
  );
}

export { LockedBalanceTooltip };
