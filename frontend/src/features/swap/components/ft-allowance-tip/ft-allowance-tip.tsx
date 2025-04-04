import { formatUnits, parseUnits } from 'viem';

import { Skeleton, Tooltip } from '@/components';
import { isUndefined } from '@/utils';

import QuestionSVG from '../../assets/question.svg?react';

import styles from './ft-allowance-tip.module.scss';

type Props = {
  allowance: bigint | undefined;
  decimals: number | undefined;
  symbol: string | undefined;
  amount: string;
  isVaraNetwork: boolean;
  isLoading: boolean;
};

function FTAllowanceTip({ allowance, decimals, symbol, amount, isVaraNetwork, isLoading }: Props) {
  const isEmpty = isUndefined(allowance) || !decimals || !symbol;

  if (isLoading || isEmpty) return <Skeleton width="16px" height="16px" disabled={!isLoading && isEmpty} />;

  const formattedAllowance = formatUnits(allowance, decimals);
  const contractName = isVaraNetwork ? 'VFT Manager' : 'ETH Bridging Payment';

  const getSubheading = () => {
    if (!allowance) return `Tokens will be approved first, followed by a transfer message.`;
    if (!amount) return 'Specify the desired transfer amount to check if additional approval is needed.';

    const parsedAmount = parseUnits(amount, decimals);

    if (parsedAmount > allowance)
      return `New value of ${amount} ${symbol} will be approved, followed by a transfer message.`;

    return `A transfer message will be sent directly to it.`;
  };

  return (
    <Tooltip
      value={
        <>
          <p className={styles.heading}>
            {allowance > 0
              ? `You have already approved ${formattedAllowance} ${symbol} to the ${contractName} contract.`
              : `You don't have any approved tokens to the ${contractName} contract yet.`}
          </p>

          <p className={styles.subheading}>{getSubheading()}</p>
        </>
      }>
      <QuestionSVG />
    </Tooltip>
  );
}

export { FTAllowanceTip };
