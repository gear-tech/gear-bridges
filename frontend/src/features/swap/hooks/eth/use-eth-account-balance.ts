import { formatEther } from 'viem';
import { useBalance } from 'wagmi';

import { useEthAccount } from '@/hooks';

const withPrecision = (value: string) => {
  // simplest solution without rounding for now
  const digitsCount = 3;

  return value.slice(0, value.indexOf('.') + digitsCount + 1);
};

function useEthAccountBalance() {
  const ethAccount = useEthAccount();

  const { data, isPending } = useBalance({
    address: ethAccount?.address,
  });

  const { value } = data || {};
  const formattedValue = data ? withPrecision(formatEther(data.value)) : undefined;
  const isLoading = isPending;

  return { value, formattedValue, isLoading };
}

export { useEthAccountBalance };
