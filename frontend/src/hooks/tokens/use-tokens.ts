import { useFTAddresses } from './use-ft-addresses';
import { useFTDecimals } from './use-ft-decimals';
import { useFTSymbols } from './use-ft-symbols';

function useTokens() {
  const { data: addresses } = useFTAddresses();
  const { data: symbols, isPending: isSymbolsPending } = useFTSymbols(addresses);
  const { data: decimals, isPending: isDecimalsPending } = useFTDecimals(addresses);

  const isLoading = isSymbolsPending || isDecimalsPending;

  return { addresses, symbols, decimals, isLoading };
}

export { useTokens };
