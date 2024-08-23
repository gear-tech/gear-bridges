import { useBalanceFormat, withoutCommas } from '@gear-js/react-hooks';

import { isUndefined } from '@/utils';

import { STATE_FUNCTION } from '../../consts';
import { ConfigState, Contract } from '../../types';

import { useReadState } from './use-read-state';

function useVaraConfig({ address, metadata }: Contract) {
  const { getFormattedBalance } = useBalanceFormat();

  const { data, isPending } = useReadState<ConfigState>(address, metadata, STATE_FUNCTION.CONFIG);

  const config = data?.Config;
  const fee = config ? BigInt(withoutCommas(config.fee)) : undefined;
  const minValue = config ? BigInt(withoutCommas(config.minAmount)) : undefined;
  const ftAddress = config?.ftTokenId;

  const formattedFee = !isUndefined(fee) ? getFormattedBalance(fee).value : undefined;
  const isLoading = isPending;

  return { fee: { value: fee, formattedValue: formattedFee }, minValue, ftAddress, isLoading };
}

export { useVaraConfig };
