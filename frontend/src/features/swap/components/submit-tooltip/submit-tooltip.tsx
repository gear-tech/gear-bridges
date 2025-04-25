import { ReactElement } from 'react';
import { formatUnits, parseUnits } from 'viem';

import { Tooltip } from '@/components';
import { isUndefined } from '@/utils';

import { useBridgeContext } from '../../context';

import styles from './submit-tooltip.module.scss';

type Props = {
  allowance: bigint | undefined;
  decimals: number | undefined;
  symbol: string | undefined;
  amount: string;
  children: ReactElement;
};

function SubmitTooltip({ allowance, decimals, symbol, amount, children }: Props) {
  const { network } = useBridgeContext();

  if (isUndefined(allowance) || !decimals || !symbol) return children;

  const formattedAllowance = formatUnits(allowance, decimals);
  const contractName = network.isVara ? 'VFT Manager' : 'ERC20 Manager';

  const getSubheading = () => {
    if (!allowance) return `Tokens will be approved first, followed by a transfer message.`;
    if (!amount) return 'Specify the desired transfer amount to check if additional approval is needed.';

    const parsedAmount = parseUnits(amount, decimals);

    if (parsedAmount > allowance)
      return `New value of ${amount} ${symbol} will be approved, followed by a transfer message.`;

    return `A transfer message will be sent directly to it.`;
  };

  const render = () => (
    <>
      <p className={styles.heading}>
        {allowance > 0
          ? `You have already approved ${formattedAllowance} ${symbol} to the ${contractName} contract.`
          : `You don't have any approved tokens to the ${contractName} contract yet.`}
      </p>

      <p className={styles.subheading}>{getSubheading()}</p>
    </>
  );

  return <Tooltip value={render()}>{children}</Tooltip>;
}

export { SubmitTooltip };
