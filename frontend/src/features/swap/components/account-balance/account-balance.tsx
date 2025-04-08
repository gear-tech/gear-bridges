import { useApi } from '@gear-js/react-hooks';
import { useEffect } from 'react';
import { formatUnits } from 'viem';

import { Skeleton, Tooltip, FormattedBalance } from '@/components';
import { isUndefined } from '@/utils';

import DangerSVG from '../../assets/danger.svg?react';
import WalletSVG from '../../assets/wallet.svg?react';
import { InsufficientAccountBalanceError } from '../../errors';
import { UseAccountBalance, UseHandleSubmit } from '../../types';

import styles from './account-balance.module.scss';

type Props = ReturnType<UseAccountBalance> & {
  submit: ReturnType<UseHandleSubmit>[0];
  isVaraNetwork: boolean;
};

function AccountBalance({ data: value, isLoading, isVaraNetwork, submit }: Props) {
  const { api } = useApi();

  const { error } = submit;
  const isBalanceError = error instanceof InsufficientAccountBalanceError;

  useEffect(() => {
    if (isUndefined(value) || !isBalanceError || value < error.requiredValue) return;

    submit.reset();

    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  if (isLoading || isUndefined(value) || !api) return <Skeleton />;

  const decimals = isVaraNetwork ? api.registry.chainDecimals[0] : 18;
  const symbol = isVaraNetwork ? 'VARA' : 'ETH';

  return (
    <div className={styles.container}>
      <div className={styles.balance}>
        <WalletSVG />
        <FormattedBalance value={value} decimals={decimals} symbol={symbol} />
      </div>

      {isBalanceError && (
        <Tooltip
          value={
            <>
              <p>{error.message}</p>

              <p>
                At least {formatUnits(error.requiredValue, decimals)} {symbol} is needed
              </p>
            </>
          }>
          <DangerSVG />
        </Tooltip>
      )}
    </div>
  );
}

export { AccountBalance };
