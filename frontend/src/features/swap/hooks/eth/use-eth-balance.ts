import { Config } from '../../types';

import { useEthAccountBalance } from './use-eth-account-balance';
import { useFungibleTokenBalance } from './use-fungible-token-balance';

function useEthBalance({ ftAddress, isLoading }: Config) {
  const accountBalance = useEthAccountBalance(!isLoading && !ftAddress);
  const fungibleTokenBalance = useFungibleTokenBalance(ftAddress);

  return ftAddress ? fungibleTokenBalance : accountBalance;
}

export { useEthBalance };
