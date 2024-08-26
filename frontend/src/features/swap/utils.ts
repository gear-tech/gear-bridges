import { getTypedEntries } from '@gear-js/react-hooks';
import { formatUnits, parseUnits } from 'viem';
import { z } from 'zod';

import { NETWORK_NAME, SPEC } from '@/consts';
import { Pair } from '@/types';
import { isUndefined } from '@/utils';

import { ERROR_MESSAGE } from './consts';

const getAmountSchema = (
  balanceValue: bigint | undefined,
  _minValue: bigint | undefined,
  feeValue: bigint | undefined,
  decimals: number | undefined,
) => {
  if (isUndefined(balanceValue) || isUndefined(_minValue) || isUndefined(feeValue) || isUndefined(decimals))
    return z.bigint();

  const minValue = _minValue + feeValue;
  const formattedMinValue = formatUnits(minValue, decimals);

  return z
    .string()
    .trim() // TODO: required field check
    .transform((value) => parseUnits(value, decimals)) // if fraction is > decimals, value will be rounded
    .refine((value) => value <= balanceValue, { message: ERROR_MESSAGE.NO_BALANCE })
    .refine((value) => value >= minValue, { message: ERROR_MESSAGE.MIN_VALUE(formattedMinValue) });
};

const getOptions = () => {
  const varaOptions: { label: string; value: Pair }[] = [];
  const ethOptions: { label: string; value: Pair }[] = [];

  getTypedEntries(SPEC).forEach(([pair, bridge]) => {
    varaOptions.push({ label: bridge[NETWORK_NAME.VARA].symbol, value: pair });
    ethOptions.push({ label: bridge[NETWORK_NAME.ETH].symbol, value: pair });
  });

  return { varaOptions, ethOptions };
};

export { getAmountSchema, getOptions };
