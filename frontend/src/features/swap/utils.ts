import { parseUnits } from 'viem';
import { z } from 'zod';

import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';

const getAmountSchema = (
  balanceValue: bigint | undefined,
  feeValue: bigint | undefined,
  decimals: number | undefined,
) => {
  if (isUndefined(balanceValue) || isUndefined(feeValue) || isUndefined(decimals)) return z.bigint();

  return z
    .string()
    .trim() // TODO: required field check
    .transform((value) => parseUnits(value, decimals)) // if fraction is > decimals, value will be rounded
    .refine((value) => value <= balanceValue, { message: ERROR_MESSAGE.NO_BALANCE });
};

export { getAmountSchema };
