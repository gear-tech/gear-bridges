import { STATE_FUNCTION } from '../../consts';
import { ConfigState, Contract } from '../../types';

import { useReadState } from './use-read-state';

function useVaraConfig({ address, sails }: Contract) {
  const { data: config, isPending } = useReadState<ConfigState>(address, sails, STATE_FUNCTION.CONFIG);

  const minValue = config ? BigInt(config.min_amount) : undefined;
  const ftAddress = BigInt(config?.ft_token_id || 0) === 0n ? undefined : config?.ft_token_id;

  const isLoading = isPending;

  return { minValue, ftAddress, isLoading };
}

export { useVaraConfig };
