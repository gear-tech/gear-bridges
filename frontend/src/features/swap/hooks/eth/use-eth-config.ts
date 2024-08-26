import { useReadContracts } from 'wagmi';

import { ABI, FUNCTION_NAME } from '../../consts';
import { Contract } from '../../types';

function useEthConfig({ address }: Contract) {
  // TODO: logger
  const abi = ABI;

  const { data, isLoading } = useReadContracts({
    contracts: [
      { abi, address, functionName: FUNCTION_NAME.MIN_AMOUNT },
      { abi, address, functionName: FUNCTION_NAME.FUNGIBLE_TOKEN_ADDRESS },
    ],
  });

  const minValue = data?.[0]?.result;
  const ftAddress = data?.[1]?.result;

  return { minValue, ftAddress, isLoading };
}

export { useEthConfig };
