import { useApi } from '@gear-js/react-hooks';
import { useEffect } from 'react';
import { formatUnits } from 'viem';

import { Skeleton, Tooltip } from '@/components';
import { useBridge } from '@/contexts';
import { isUndefined } from '@/utils';

import DangerSVG from '../../assets/danger.svg?react';
import WalletSVG from '../../assets/wallet.svg?react';
import { InsufficientAccountBalanceError } from '../../errors';
import { UseAccountBalance, UseHandleSubmit } from '../../types';

import styles from './account-balance.module.scss';

type Props = ReturnType<UseAccountBalance> & {
  amount: string;
  submit: ReturnType<UseHandleSubmit>[0];
  isVaraNetwork: boolean;
};

function AccountBalance({ value, amount, formattedValue, isLoading, isVaraNetwork, submit }: Props) {
  const { api } = useApi();
  const { pairIndex } = useBridge();

  const { error } = submit;
  const isBalanceError = error instanceof InsufficientAccountBalanceError;

  useEffect(() => {
    submit.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pairIndex, amount]);

  useEffect(() => {
    if (isUndefined(value) || !isBalanceError || value < error.requiredValue) return;

    submit.reset();

    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [value]);

  if (isLoading || !formattedValue || !api) return <Skeleton />;

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
