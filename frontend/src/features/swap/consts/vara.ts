const SERVICE_NAME = {
  BRIDGING_PAYMENT: 'bridgingPayment',
  VFT_GATEWAY: 'vftGateway',
  VFT: 'vft',
} as const;

const FUNCTION_NAME = {
  REQUEST_TO_GATEWAY: 'requestToGateway',
  APPROVE: 'approve',
} as const;

const QUERY_NAME = {
  VFT_GATEWAY_ADDRESS: 'vftGatewayAddress',
  FT_ADDRESSES: 'varaToEthAddresses',
  BALANCE: 'balanceOf',
  DECIMALS: 'decimals',
  GET_CONFIG: 'getConfig',
} as const;

export { SERVICE_NAME, FUNCTION_NAME, QUERY_NAME };