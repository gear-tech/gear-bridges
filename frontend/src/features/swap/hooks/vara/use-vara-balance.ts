import { Config } from '../../types';

import { useFungibleTokenBalance } from './use-fungible-token-balance';
import { useVaraAccountBalance } from './use-vara-account-balance';

function useVaraBalance({ ftAddress, isLoading }: Config) {
  const accountBalance = useVaraAccountBalance(!isLoading && !ftAddress);
  const fungibleTokenBalance = useFungibleTokenBalance(ftAddress);

  return ftAddress ? fungibleTokenBalance : accountBalance;
}

export { useVaraBalance };
