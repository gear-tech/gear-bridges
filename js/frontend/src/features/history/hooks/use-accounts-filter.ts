import { useAccount } from '@gear-js/react-hooks';

import { useEthAccount } from '@/hooks';

function useAccountsFilter() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();

  const addresses = [account?.decodedAddress, ethAccount.address?.toLowerCase()].filter((value) => Boolean(value));
  const isAvailable = addresses.length > 0;

  return { addresses, isAvailable };
}

export { useAccountsFilter };
