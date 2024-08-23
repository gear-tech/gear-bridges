import { useAccount, useApi, useBalanceFormat } from '@gear-js/react-hooks';
import { useMemo } from 'react';

import { useDeriveBalancesAll } from './use-derive-balances-all';

function useVaraAccountBalance(isEnabled: boolean) {
  const { api } = useApi();
  const { account, isAccountReady } = useAccount();
  const { data, isPending } = useDeriveBalancesAll(isEnabled ? account?.address : undefined);
  const { getFormattedBalance } = useBalanceFormat();

  const { freeBalance } = data || {};
  const value = freeBalance?.toBigInt();
  const formattedValue = value ? getFormattedBalance(value).value : undefined;

  // cuz swap vara form is rendered by default without login and we have to handle empty balance state
  const isLoading = useMemo(() => {
    if (!isAccountReady) return true;
    if (!account) return false;

    return isPending;
  }, [account, isAccountReady, isPending]);

  const [decimals] = api?.registry.chainDecimals || [undefined];

  return { value, formattedValue, decimals, isLoading };
}

export { useVaraAccountBalance };
