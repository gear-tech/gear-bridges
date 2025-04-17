import { parseUnits } from 'viem';
import { z } from 'zod';

import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';
import { UseAccountBalance } from './types';

const getAmountSchema = (
  isNativeToken: boolean,
  accountBalanceValue: bigint | undefined,
  ftBalanceValue: bigint | undefined,
  decimals: number | undefined,
) => {
  if (isUndefined(accountBalanceValue) || isUndefined(ftBalanceValue) || isUndefined(decimals))
    return z.string().transform((value) => BigInt(value));

  const schema = z
    .string()
    .trim()
    .transform((value) => parseUnits(value, decimals)) // if fraction is > decimals, value will be rounded
    .refine((value) => value > 0n, { message: ERROR_MESSAGE.MIN_AMOUNT });

  if (!isNativeToken)
    return schema.refine((value) => value <= ftBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE });

  return schema.refine((value) => value <= accountBalanceValue + ftBalanceValue, {
    message: ERROR_MESSAGE.NO_FT_BALANCE,
  });
};

const getMergedBalance = (accountBalance: ReturnType<UseAccountBalance>, ftBalance: ReturnType<UseAccountBalance>) => {
  const isLoading = accountBalance.isLoading || ftBalance.isLoading;

  const data =
    !isUndefined(accountBalance.data) && !isUndefined(ftBalance.data)
      ? accountBalance.data + ftBalance.data
      : undefined;

  return { data, isLoading };
};

export { getAmountSchema, getMergedBalance };
