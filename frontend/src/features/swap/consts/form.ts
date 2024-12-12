import { decodeAddress } from '@gear-js/api';
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
  MIN_AMOUNT: 'Amount is less than fee',
} as const;

const VARA_ADDRESS_SCHEMA = z
  .string()
  .trim()
  .refine((value) => isSubstrateAddress(value), { message: ERROR_MESSAGE.INVALID_ADDRESS })
  .transform((value) => decodeAddress(value));

const ETH_ADDRESS_SCHEMA = z
  .string()
  .trim()
  .refine((value) => isEthAddress(value), { message: ERROR_MESSAGE.INVALID_ADDRESS });

const ADDRESS_SCHEMA = {
  VARA: VARA_ADDRESS_SCHEMA,
  ETH: ETH_ADDRESS_SCHEMA,
};

export { FIELD_NAME, ERROR_MESSAGE, DEFAULT_VALUES, ADDRESS_SCHEMA };
