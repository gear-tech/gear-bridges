import { useAccount, useDeriveBalancesAll } from '@gear-js/react-hooks';

function useVaraAccountBalance() {
  const { account } = useAccount();

  return useDeriveBalancesAll({
    address: account?.address,
    watch: true,
    query: {
      select: (value) => (value.transferable || value.availableBalance).toBigInt(),
    },
  });
}

export { useVaraAccountBalance };
