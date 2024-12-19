import { useBalance } from 'wagmi';

import { useInvalidateOnBlock } from './common';
import { useEthAccount } from './use-eth-account';

function useEthAccountBalance() {
  const ethAccount = useEthAccount();

  const state = useBalance({
    address: ethAccount?.address,
    query: { select: ({ value }) => value },
  });

  const { queryKey } = state;
  useInvalidateOnBlock({ queryKey });

  return state;
}

export { useEthAccountBalance };
