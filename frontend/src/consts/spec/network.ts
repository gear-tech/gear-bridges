const NETWORK_NAME = {
  VARA: 'vara',
  ETH: 'ethereum',
} as const;

const NATIVE_SYMBOL = {
  VARA: 'VARA',
  ETH: 'ETH',
} as const;

const FEE_DECIMALS = {
  [NETWORK_NAME.VARA]: 12,
  [NETWORK_NAME.ETH]: 18,
} as const;

const NETWORK_NATIVE_SYMBOL = {
  [NETWORK_NAME.VARA]: NATIVE_SYMBOL.VARA,
  [NETWORK_NAME.ETH]: NATIVE_SYMBOL.ETH,
} as const;

export { NETWORK_NAME, NATIVE_SYMBOL, FEE_DECIMALS, NETWORK_NATIVE_SYMBOL };
