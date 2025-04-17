import { useAccount } from '@gear-js/react-hooks';

import WarningSVG from '@/assets/warning.svg?react';
import { Tooltip } from '@/components';

import { useTransactionsCount } from '../../hooks';
import { Status, TransferWhereInput } from '../../types';

function PendingTransactionsTooltip() {
  const { account } = useAccount(); // fee payment is a standalone transaction only for vara network

  const [txsCount] = useTransactionsCount(
    account ? ({ sender_eq: account.decodedAddress, status_eq: Status.Pending } as TransferWhereInput) : undefined,
  );

  if (!account || !txsCount) return;

  return (
    <Tooltip value="You have transactions awaiting fee payment">
      <WarningSVG />
    </Tooltip>
  );
}

export { PendingTransactionsTooltip };
