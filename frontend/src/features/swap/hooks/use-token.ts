import { useTokens } from '@/hooks';

function useToken(networkIndex: number, pairIndex: number) {
  const { addresses, symbols, decimals: tokenDecimals, isLoading } = useTokens();

  const address = addresses?.[pairIndex][networkIndex];
  const destinationAddress = addresses?.[pairIndex][Number(!networkIndex)];
  const symbol = address ? symbols?.[address] : undefined;
  const destinationSymbol = destinationAddress ? symbols?.[destinationAddress] : undefined;
  const decimals = address ? tokenDecimals?.[address] : undefined;

  return { address, destinationAddress, destinationSymbol, symbol, decimals, isLoading };
}

export { useToken };
