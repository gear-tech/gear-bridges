import { parseUnits } from 'viem';
import { z } from 'zod';

import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';

const getAmountSchema = (
  accountBalanceValue: bigint | undefined,
  ftBalanceValue: bigint | undefined,
  feeValue: bigint | undefined,
  decimals: number | undefined,
) => {
  if (isUndefined(accountBalanceValue) || isUndefined(ftBalanceValue) || isUndefined(feeValue) || isUndefined(decimals))
    return z.bigint();

  return z
    .string()
    .trim() // TODO: required field check
    .transform((value) => parseUnits(value, decimals)) // if fraction is > decimals, value will be rounded
    .refine((value) => value <= ftBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE })
    .refine(() => feeValue <= accountBalanceValue, { message: ERROR_MESSAGE.NO_ACCOUNT_BALANCE });
};

const getOptions = (symbols: [string, string][] | undefined) => {
  const varaOptions: { label: string; value: string }[] = [];
  const ethOptions: { label: string; value: string }[] = [];

  if (!symbols) return { varaOptions, ethOptions };

  symbols.forEach(([varaSymbol, ethSymbol], index) => {
    const value = index.toString();

    varaOptions.push({ label: varaSymbol, value });
    ethOptions.push({ label: ethSymbol, value });
  });

  return { varaOptions, ethOptions };
};

export { getAmountSchema, getOptions };
