import { useAccount, useBalanceFormat, useDeriveBalancesAll } from '@gear-js/react-hooks';
import { useMemo } from 'react';

function useVaraAccountBalance() {
  const { account, isAccountReady } = useAccount();
  const { getFormattedBalance } = useBalanceFormat();

  const { data } = useDeriveBalancesAll({ address: account?.address, watch: true });
  const { transferable, availableBalance } = data || {};
  const value = (transferable || availableBalance)?.toBigInt();
  const formattedValue = value !== undefined ? getFormattedBalance(value).value : undefined;

  // cuz swap vara form is rendered by default without login and we have to handle empty balance state
  const isLoading = useMemo(() => {
    if (!isAccountReady) return true;
    if (!account) return false;

    return !data;
  }, [account, isAccountReady, data]);

  return { value, formattedValue, isLoading };
}

export { useVaraAccountBalance };
