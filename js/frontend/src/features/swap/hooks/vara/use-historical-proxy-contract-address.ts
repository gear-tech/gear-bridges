import { useProgramQuery } from '@gear-js/react-hooks';

import { useVFTManagerProgram } from './use-vft-manager-program';

function useHistoricalProxyContractAddress() {
  const { data: program } = useVFTManagerProgram();

  return useProgramQuery({
    program,
    serviceName: 'vftManager',
    functionName: 'historicalProxyAddress',
    args: [],
  });
}

export { useHistoricalProxyContractAddress };
