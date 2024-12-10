import { HexString } from '@gear-js/api';
import { formatUnits, parseUnits } from 'viem';
import { z } from 'zod';

import { FTAddressPair } from '@/types';
import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';
import { UseAccountBalance } from './types';

const getAmountSchema = (
  isNativeToken: boolean,
  accountBalanceValue: bigint | undefined,
  ftBalanceValue: bigint | undefined,
  feeValue: bigint | undefined,
  decimals: number | undefined,
) => {
  if (isUndefined(accountBalanceValue) || isUndefined(ftBalanceValue) || isUndefined(feeValue) || isUndefined(decimals))
    return z.bigint();

  const schema = z
    .string()
    .trim() // TODO: required field check
    .transform((value) => parseUnits(value, decimals)); // if fraction is > decimals, value will be rounded

  if (!isNativeToken)
    return schema
      .refine((value) => value <= ftBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE })
      .refine(() => feeValue <= accountBalanceValue, { message: ERROR_MESSAGE.NO_ACCOUNT_BALANCE });

  return schema
    .refine((value) => value >= feeValue, { message: ERROR_MESSAGE.MIN_AMOUNT })
    .refine(
      (value) => {
        const expectedValue = value - feeValue;
        const isMintRequired = expectedValue > ftBalanceValue;
        const valueToMint = isMintRequired ? expectedValue - ftBalanceValue : BigInt(0);

        return valueToMint + feeValue <= accountBalanceValue;
      },
      { message: ERROR_MESSAGE.NO_ACCOUNT_BALANCE },
    );
};

const getOptions = (addresses: FTAddressPair[] | undefined, symbols: Record<HexString, string> | undefined) => {
  const varaOptions: { label: string; value: string }[] = [];

  if (!addresses || !symbols) return [];

  addresses.forEach(([varaAddress], index) => {
    const value = index.toString();
    const label = symbols[varaAddress];

    varaOptions.push({ label, value });
  });

  return varaOptions;
};

const getMergedBalance = (
  accountBalance: ReturnType<UseAccountBalance>,
  ftBalance: ReturnType<UseAccountBalance>,
  decimals: number | undefined,
) => {
  const isLoading = accountBalance.isLoading || ftBalance.isLoading;

  if (
    isUndefined(accountBalance.value) ||
    isUndefined(ftBalance.value) ||
    isUndefined(decimals) ||
    !accountBalance.formattedValue ||
    !ftBalance.formattedValue
  )
    return { value: undefined, formattedValue: undefined, isLoading };

  const value = accountBalance.value + ftBalance.value;
  const formattedValue = formatUnits(value, decimals);

  return { value, formattedValue, isLoading };
};

export { getAmountSchema, getOptions, getMergedBalance };
