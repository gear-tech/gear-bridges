import { useAlert } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { formatUnits, parseUnits } from 'viem';
import { z } from 'zod';

import { isUndefined } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { FormattedValues } from '../types';
import { getAmountSchema } from '../utils';

type Values = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

function useSwapForm(
  isVaraNetwork: boolean,
  balance: Values & { decimals: number | undefined },
  fee: bigint | undefined,
  minValue: bigint | undefined,
  disabled: boolean,
  onSubmit: (values: FormattedValues, reset: () => void) => void,
) {
  const { decimals } = balance;

  const alert = useAlert();

  const valueSchema = getAmountSchema(balance.value, minValue, fee, decimals);
  const expectedValueSchema = getAmountSchema(balance.value, minValue, BigInt(0), decimals);

  const addressSchema = isVaraNetwork ? ADDRESS_SCHEMA.ETH : ADDRESS_SCHEMA.VARA;

  const schema = z.object({
    [FIELD_NAME.VALUE]: valueSchema,
    [FIELD_NAME.EXPECTED_VALUE]: expectedValueSchema,
    [FIELD_NAME.ADDRESS]: addressSchema,
  });

  const form = useForm<typeof DEFAULT_VALUES, unknown, z.infer<typeof schema>>({
    defaultValues: DEFAULT_VALUES,
    resolver: zodResolver(schema),
  });

  const { setValue, reset, formState } = form;
  const shouldValidate = formState.isSubmitted; // validating only if validation was already fired

  const setOriginalValue = (value: string) => setValue(FIELD_NAME.VALUE, value, { shouldValidate });
  const setExpectedValue = (value: string) => setValue(FIELD_NAME.EXPECTED_VALUE, value, { shouldValidate });

  const getValueWithFee = (value: string, operator: '+' | '-' = '+') => {
    if (isUndefined(fee)) throw new Error('Fee is not defined');
    if (isUndefined(decimals)) throw new Error('Decimals is not defined');
    if (!value) return value;

    const chainValue = parseUnits(value, decimals);
    const valueWithFee = operator === '+' ? chainValue + fee : chainValue - fee;

    return valueWithFee < 0 ? '0' : formatUnits(valueWithFee, decimals);
  };

  const onValueChange = (value: string) => setExpectedValue(getValueWithFee(value, '-'));
  const onExpectedValueChange = (value: string) => setOriginalValue(getValueWithFee(value));

  const handleSubmit = form.handleSubmit((values) => {
    const onSuccess = () => {
      reset();
      alert.success('Successful transaction');
    };

    onSubmit(values, onSuccess);
  });

  useEffect(() => {
    reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);

  const setMaxBalance = () => {
    if (!balance.formattedValue) throw new Error('Balance is not defined');

    setOriginalValue(balance.formattedValue);
    onValueChange(balance.formattedValue);
  };

  return {
    form,
    onValueChange,
    onExpectedValueChange,
    handleSubmit,
    setMaxBalance,
  };
}

export { useSwapForm };
