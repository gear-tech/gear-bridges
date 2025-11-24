import { formatUnits, parseUnits } from 'viem';
import { z } from 'zod';

import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';

const getAmountSchema = (
  isNativeToken: boolean | undefined,
  accountBalanceValue: bigint | undefined,
  ftBalanceValue: bigint | undefined,
  decimals: number | undefined,
  eDeposit: bigint | undefined,
) => {
  if (
    isUndefined(isNativeToken) ||
    isUndefined(accountBalanceValue) ||
    isUndefined(ftBalanceValue) ||
    isUndefined(decimals) ||
    isUndefined(eDeposit)
  )
    return;

  const schema = z
    .string()
    .trim()
    .transform((value) => parseUnits(value, decimals)); // if fraction is > decimals, value will be rounded

  if (isNativeToken) {
    // vft balance is like wallet balance,
    // existential deposit is a minimum required amount to transfer if balance is 0
    const minAmountMsg = eDeposit ? `Minimum amount is ${formatUnits(eDeposit, decimals)}` : ERROR_MESSAGE.MIN_AMOUNT;

    return schema
      .refine((value) => (eDeposit ? value >= eDeposit : value > 0n), { message: minAmountMsg })
      .refine((value) => value <= accountBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE });
  }

  return schema
    .refine((value) => value > 0n, { message: ERROR_MESSAGE.MIN_AMOUNT })
    .refine((value) => value <= ftBalanceValue, { message: ERROR_MESSAGE.NO_FT_BALANCE });
};

const estimateBridging = (txs: { gasLimit: bigint; value?: bigint }[], valuePerGas: bigint) => {
  const totalGasLimit = txs.reduce((sum, { gasLimit }) => sum + gasLimit, 0n) * valuePerGas;
  const totalValue = txs.reduce((sum, { value = 0n }) => sum + value, 0n);

  return { totalGasLimit, totalValue };
};

export { getAmountSchema, estimateBridging };
