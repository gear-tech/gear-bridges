import { useAlert } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { z } from 'zod';

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
  disabled: boolean,
  onSubmit: (values: FormattedValues, reset: () => void) => void,
) {
  const alert = useAlert();

  const valueSchema = getAmountSchema(balance.value, fee, balance.decimals);
  const expectedValueSchema = getAmountSchema(balance.value, BigInt(0), balance.decimals);
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

  const onValueChange = (value: string) => setExpectedValue(value);
  const onExpectedValueChange = (value: string) => setOriginalValue(value);

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
