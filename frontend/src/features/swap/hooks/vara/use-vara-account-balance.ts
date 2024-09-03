import { useAccount, useBalanceFormat } from '@gear-js/react-hooks';
import { useMemo } from 'react';

import { useDeriveBalancesAll } from './use-derive-balances-all';

function useVaraAccountBalance() {
  const { account, isAccountReady } = useAccount();
  const { getFormattedBalance } = useBalanceFormat();

  const { data, isPending } = useDeriveBalancesAll(account?.address);
  const { freeBalance } = data || {};
  const value = freeBalance?.toBigInt();
  const formattedValue = value !== undefined ? getFormattedBalance(value).value : undefined;

  // cuz swap vara form is rendered by default without login and we have to handle empty balance state
  const isLoading = useMemo(() => {
    if (!isAccountReady) return true;
    if (!account) return false;

    return isPending;
  }, [account, isAccountReady, isPending]);

  return { value, formattedValue, isLoading };
}

export { useVaraAccountBalance };
