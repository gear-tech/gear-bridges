import WarningSVG from '@/assets/warning.svg?react';
import { Tooltip } from '@/components';
import { useTokens } from '@/context';
import { useEthFTBalance, useVaraFTBalance } from '@/hooks';

function LockedBalanceTooltip() {
  const { nativeToken } = useTokens();

  const { data: lockedVaraBalance } = useVaraFTBalance(nativeToken.vara?.address);
  const { data: lockedEthBalance } = useEthFTBalance(nativeToken.eth?.address);
  const lockedBalance = lockedVaraBalance || lockedEthBalance;

  if (!lockedBalance) return;

  return (
    <Tooltip value="You have tokens available to unlock">
      <WarningSVG />
    </Tooltip>
  );
}

export { LockedBalanceTooltip };
