import { HexString } from '@gear-js/api';
import { ActorId, H160 } from 'sails-js';
import { formatUnits, parseUnits } from 'viem';
import { z } from 'zod';

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

  return schema.refine(
    (value) => {
      const isMintRequired = value > ftBalanceValue;
      const valueToMint = isMintRequired ? value - ftBalanceValue : BigInt(0);

      return valueToMint <= accountBalanceValue;
    },
    { message: ERROR_MESSAGE.NO_ACCOUNT_BALANCE },
  );
};

const getOptions = (
  addresses: [ActorId, H160, 'ethereum' | 'gear'][] | undefined,
  symbols: Record<HexString, string> | undefined,
) => {
  const varaOptions: { label: string; value: string }[] = [];
  const ethOptions: { label: string; value: string }[] = [];

  if (!addresses || !symbols) return { varaOptions, ethOptions };

  addresses.forEach((pair, index) => {
    const value = index.toString();

    const varaAddress = pair[0].toString() as HexString;
    const ethAddress = pair[1].toString() as HexString;

    const varaSymbol = symbols[varaAddress];
    const ethSymbol = symbols[ethAddress];

    varaOptions.push({ label: varaSymbol, value });
    ethOptions.push({ label: ethSymbol, value });
  });

  return { varaOptions, ethOptions };
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
