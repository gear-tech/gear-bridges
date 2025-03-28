import { ExtrinsicFailedData } from '@gear-js/api';
import { BaseError, parseUnits } from 'viem';
import { WriteContractErrorType } from 'wagmi/actions';
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
  if (isUndefined(accountBalanceValue) || isUndefined(ftBalanceValue) || isUndefined(decimals)) return z.bigint();

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

// string is only for cancelled sign and send popup error during useSendProgramTransaction
// reevaluate after @gear-js/react-hooks update
const getErrorMessage = (error: Error | WriteContractErrorType | ExtrinsicFailedData | string) => {
  if (typeof error === 'object' && 'docs' in error) {
    return error.docs || error.method || error.name;
  }

  return typeof error === 'string' ? error : (error as BaseError).shortMessage || error.message;
};

export { getAmountSchema, getMergedBalance, getErrorMessage };
