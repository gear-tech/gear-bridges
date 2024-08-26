import { HexString } from '@gear-js/api';

import { useFungibleTokenBalance } from './use-fungible-token-balance';
import { useVaraAccountBalance } from './use-vara-account-balance';

function useVaraBalance(ftAddress: HexString | undefined, isLoading: boolean) {
  const accountBalance = useVaraAccountBalance(!isLoading && !ftAddress);
  const fungibleTokenBalance = useFungibleTokenBalance(ftAddress);

  return ftAddress ? fungibleTokenBalance : accountBalance;
}

export { useVaraBalance };
