import { decodeAddress, HexString } from '@gear-js/api';
import { isAddress as isEthAddress } from 'viem';
import { z } from 'zod';

import { isValidAddress as isSubstrateAddress } from '@/utils';

const FIELD_NAME = {
  VALUE: 'amount',
  ADDRESS: 'accountAddress',
} as const;

const DEFAULT_VALUES = {
  [FIELD_NAME.VALUE]: '',
  [FIELD_NAME.ADDRESS]: '',
};

const ERROR_MESSAGE = {
  NO_FT_BALANCE: 'Insufficient token balance',
  INVALID_ADDRESS: 'Invalid address',
  MIN_AMOUNT: 'Amount should be bigger than 0',
} as const;

const VARA_ADDRESS_SCHEMA = z
  .string()
  .trim()
  .refine((value) => isSubstrateAddress(value), { message: ERROR_MESSAGE.INVALID_ADDRESS })
  .transform((value) => decodeAddress(value).toLowerCase() as HexString);

const ETH_ADDRESS_SCHEMA = z
  .string()
  .trim()
  .refine((value) => isEthAddress(value), { message: ERROR_MESSAGE.INVALID_ADDRESS })
  .transform((value) => value.toLowerCase() as HexString);

const ADDRESS_SCHEMA = {
  VARA: VARA_ADDRESS_SCHEMA,
  ETH: ETH_ADDRESS_SCHEMA,
};

const SUBMIT_STATUS = {
  SUCCESS: 'success',
  BRIDGE: 'bridge',
  FEE: 'fee',
  MINT: 'mint',
  APPROVE: 'approve',
  PERMIT: 'permit',
} as const;

export { FIELD_NAME, ERROR_MESSAGE, DEFAULT_VALUES, ADDRESS_SCHEMA, SUBMIT_STATUS };
