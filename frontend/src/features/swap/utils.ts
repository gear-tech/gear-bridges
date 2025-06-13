import { formatUnits, parseUnits } from 'viem';
import { z } from 'zod';

import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';

const getAmountSchema = (
  isNativeToken: boolean | undefined,
  accountBalanceValue: bigint | undefined,
  ftBalanceValue: bigint | undefined,
  decimals: number | undefined,
  existentialDeposit: bigint | undefined,
) => {
  if (
    isUndefined(isNativeToken) ||
    isUndefined(accountBalanceValue) ||
    isUndefined(ftBalanceValue) ||
    isUndefined(decimals) ||
    isUndefined(existentialDeposit)
  )
    return z.string().transform((value) => BigInt(value));

  const schema = z
    .string()
    .trim()
    .transform((value) => parseUnits(value, decimals)); // if fraction is > decimals, value will be rounded

  if (!isNativeToken)
    return schema
      .refine((value) => value > 0n, { message: ERROR_MESSAGE.MIN_AMOUNT })
      .refine((value) => value <= ftBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE });

  return schema
    .refine(
      // vft balance is like wallet balance,
      // existentialDeposit is a minimum required amount to transfer if balance is 0
      (value) => value >= existentialDeposit,
      { message: `Minimum amount is ${formatUnits(existentialDeposit, decimals)}` },
    )
    .refine((value) => value <= accountBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE });
};

export { getAmountSchema };
