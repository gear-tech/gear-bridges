import WarningSVG from '@/assets/warning.svg?react';
import { Tooltip } from '@/components';
import { useTokens } from '@/context';
import { useEthFTBalance, useVaraFTBalance } from '@/hooks';

function LockedBalanceTooltip() {
  const { tokens } = useTokens();

  // TODO: active filter
  const { data: lockedVaraBalance } = useVaraFTBalance(
    tokens?.find(({ network, isActive, isNative }) => isActive && isNative && network === 'vara')?.address,
  );

  const { data: lockedEthBalance } = useEthFTBalance(
    tokens?.find(({ network, isActive, isNative }) => isActive && isNative && network === 'eth')?.address,
  );

  const lockedBalance = lockedVaraBalance || lockedEthBalance;

  if (!lockedBalance) return;

  return (
    <Tooltip value="You have tokens available to unlock">
      <WarningSVG />
    </Tooltip>
  );
}

export { LockedBalanceTooltip };
