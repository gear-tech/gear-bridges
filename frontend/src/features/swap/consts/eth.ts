const FUNCTION_NAME = {
  TRANSIT: 'transitEthToVara',
  FEE: 'fee',
  MIN_AMOUNT: 'minAmount',

  FUNGIBLE_TOKEN_ADDRESS: 'addressOfToken',
  FUNGIBLE_TOKEN_BALANCE: 'balanceOf', // fungible token abi
  FUNGIBLE_TOKEN_APPROVE: 'approve', // fungible token abi
  FUNGIBLE_TOKEN_DECIMALS: 'decimals', // fungible token abi
} as const;

const EVENT_NAME = {
  APPROVAL: 'Approval',
} as const;

export { FUNCTION_NAME, EVENT_NAME };
