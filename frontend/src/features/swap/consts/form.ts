import { getVaraAddress } from '@gear-js/react-hooks';
import { isAddress as isEthAddress } from 'viem';
import { z } from 'zod';

import { isValidAddress as isSubstrateAddress } from '@/utils';

const FIELD_NAME = {
  VALUE: 'amount',
  EXPECTED_VALUE: 'expectedAmount',
  ADDRESS: 'accountAddress',
} as const;

const DEFAULT_VALUES = {
  [FIELD_NAME.VALUE]: '',
  [FIELD_NAME.EXPECTED_VALUE]: '',
  [FIELD_NAME.ADDRESS]: '',
};

const ERROR_MESSAGE = {
  NO_BALANCE: 'Insufficient balance',
  INVALID_ADDRESS: 'Invalid address',
  MIN_VALUE: (value: string) => `Minimum value is ${value}`,
} as const;

const VARA_ADDRESS_SCHEMA = z
  .string()
  .trim()
  .refine((value) => isSubstrateAddress(value), { message: ERROR_MESSAGE.INVALID_ADDRESS })
  .transform((value) => getVaraAddress(value));

const ETH_ADDRESS_SCHEMA = z
  .string()
  .trim()
  .refine((value) => isEthAddress(value), { message: ERROR_MESSAGE.INVALID_ADDRESS })
  .transform((value) => value.replace('0x', ''));

const ADDRESS_SCHEMA = {
  VARA: VARA_ADDRESS_SCHEMA,
  ETH: ETH_ADDRESS_SCHEMA,
};

export { FIELD_NAME, ERROR_MESSAGE, DEFAULT_VALUES, ADDRESS_SCHEMA };
