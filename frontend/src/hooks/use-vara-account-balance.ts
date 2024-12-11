import { useAccount, useBalanceFormat, useDeriveBalancesAll } from '@gear-js/react-hooks';

function useVaraAccountBalance() {
  const { account } = useAccount();
  const { getFormattedBalance } = useBalanceFormat();

  const { data, isLoading } = useDeriveBalancesAll({ address: account?.address, watch: true });
  const { transferable, availableBalance } = data || {};
  const value = (transferable || availableBalance)?.toBigInt();
  const formattedValue = value !== undefined ? getFormattedBalance(value).value : undefined;

  return { value, formattedValue, isLoading };
}

export { useVaraAccountBalance };
