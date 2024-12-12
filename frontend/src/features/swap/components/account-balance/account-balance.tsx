import { useApi, useBalanceFormat } from '@gear-js/react-hooks';
import { formatUnits } from 'viem';

import { Skeleton, Tooltip } from '@/components';

import DangerSVG from '../../assets/danger.svg?react';
import WalletSVG from '../../assets/wallet.svg?react';
import { InsufficientAccountBalanceError } from '../../errors';
import { UseAccountBalance, UseHandleSubmit } from '../../types';

import styles from './account-balance.module.scss';

type Props = ReturnType<UseAccountBalance> & {
  submit: ReturnType<UseHandleSubmit>[0];
  isVaraNetwork: boolean;
};

function AccountBalance({ value, formattedValue, isLoading, isVaraNetwork, submit }: Props) {
  const { api } = useApi();

  if (isLoading || !formattedValue || !api) return <Skeleton />;

  const { error } = submit;
  const isBalanceError = error instanceof InsufficientAccountBalanceError;
  const decimals = isVaraNetwork ? api.registry.chainDecimals[0] : 18;
  const symbol = isVaraNetwork ? 'VARA' : 'ETH';

  return (
    <div className={styles.container}>
      <div className={styles.balance}>
        <WalletSVG />
        {`${formattedValue} ${symbol}`}
      </div>

      {isBalanceError && (
        <Tooltip SVG={DangerSVG}>
          <p>{error.message}</p>

          <p>
            At least {formatUnits(error.requiredValue, decimals)} {symbol} is needed
          </p>
        </Tooltip>
      )}
    </div>
  );
}

export { AccountBalance };
