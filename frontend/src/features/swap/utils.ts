import { HexString } from '@gear-js/api';
import { BaseError, formatUnits, parseUnits } from 'viem';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { FTAddressPair } from '@/types';
import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';
import { UseAccountBalance } from './types';

const getAmountSchema = (
  isNativeToken: boolean,
  accountBalanceValue: bigint | undefined,
  ftBalanceValue: bigint | undefined,
  decimals: number | undefined,
) => {
  if (isUndefined(accountBalanceValue) || isUndefined(ftBalanceValue) || isUndefined(decimals)) return z.bigint();

  const schema = z
    .string()
    .trim() // TODO: required field check
    .transform((value) => parseUnits(value, decimals)) // if fraction is > decimals, value will be rounded
    .refine((value) => value > 0n, { message: ERROR_MESSAGE.MIN_AMOUNT });

  if (!isNativeToken)
    return schema.refine((value) => value <= ftBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE });

  return schema.refine((value) => value <= accountBalanceValue + ftBalanceValue, {
    message: ERROR_MESSAGE.NO_FT_BALANCE,
  });
};

const getOptions = (addresses: FTAddressPair[] | undefined, symbols: Record<HexString, string> | undefined) => {
  const varaOptions: { label: string; value: string }[] = [];
  const ethOptions: { label: string; value: string }[] = [];

  if (!addresses || !symbols) return { varaOptions, ethOptions };

  addresses.forEach(([varaAddress, ethAddress], pairIndex) => {
    const value = pairIndex.toString();
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

// string is only for cancelled sign and send popup error during useSendProgramTransaction
// reevaluate after @gear-js/react-hooks update
const getErrorMessage = (error: Error | WriteContractErrorType | string) =>
  typeof error === 'string' ? error : (error as BaseError).shortMessage || error.message;

export { getAmountSchema, getOptions, getMergedBalance, getErrorMessage };
