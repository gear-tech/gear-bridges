import WarningSVG from '@/assets/warning.svg?react';
import { Tooltip } from '@/components';
import { useEthFTBalance, useTokens, useVaraFTBalance } from '@/hooks';

function LockedBalanceTooltip() {
  const { wrappedVaraAddress, wrappedEthAddress } = useTokens();

  const { data: lockedVaraBalance } = useVaraFTBalance(wrappedVaraAddress);
  const { data: lockedEthBalance } = useEthFTBalance(wrappedEthAddress);
  const lockedBalance = lockedVaraBalance || lockedEthBalance;

  if (!lockedBalance) return;

  return (
    <Tooltip value="You have tokens available to unlock">
      <WarningSVG />
    </Tooltip>
  );
}

export { LockedBalanceTooltip };
