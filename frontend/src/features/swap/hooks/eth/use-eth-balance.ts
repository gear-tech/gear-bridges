import { HexString } from '@gear-js/api';

import { useEthAccountBalance } from './use-eth-account-balance';
import { useFungibleTokenBalance } from './use-fungible-token-balance';

function useEthBalance(ftAddress: HexString | undefined, isLoading: boolean) {
  const accountBalance = useEthAccountBalance(!isLoading && !ftAddress);
  const fungibleTokenBalance = useFungibleTokenBalance(ftAddress);

  return ftAddress ? fungibleTokenBalance : accountBalance;
}

export { useEthBalance };
