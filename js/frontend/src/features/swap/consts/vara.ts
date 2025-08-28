const SERVICE_NAME = {
  BRIDGING_PAYMENT: 'bridgingPayment',
  VFT: 'vft',
} as const;

const FUNCTION_NAME = {
  APPROVE: 'approve',
} as const;

const QUERY_NAME = {
  FT_ADDRESSES: 'varaToEthAddresses',
  BALANCE: 'balanceOf',
  DECIMALS: 'decimals',
  GET_CONFIG: 'getConfig',
} as const;

export { SERVICE_NAME, FUNCTION_NAME, QUERY_NAME };
