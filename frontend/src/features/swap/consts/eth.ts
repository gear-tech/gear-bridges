const FUNCTION_NAME = {
  DEPOSIT: 'deposit',

  FUNGIBLE_TOKEN_BALANCE: 'balanceOf', // fungible token abi
  FUNGIBLE_TOKEN_APPROVE: 'approve', // fungible token abi
  FUNGIBLE_TOKEN_DECIMALS: 'decimals', // fungible token abi
} as const;

const EVENT_NAME = {
  APPROVAL: 'Approval',
} as const;

export { FUNCTION_NAME, EVENT_NAME };
