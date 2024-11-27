import { useAlert } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { BaseError } from 'wagmi';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { logger } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { FormattedValues } from '../types';
import { getAmountSchema } from '../utils';

type Values = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

function useSwapForm(
  isVaraNetwork: boolean,
  accountBalance: Values,
  ftBalance: Values,
  decimals: number | undefined,
  fee: bigint | undefined,
  disabled: boolean,
  onSubmit: (values: FormattedValues) => Promise<unknown>,
) {
  const alert = useAlert();

  const valueSchema = getAmountSchema(accountBalance.value, ftBalance.value, fee, decimals);
  const expectedValueSchema = getAmountSchema(accountBalance.value, ftBalance.value, BigInt(0), decimals);
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
  const amount = form.watch(FIELD_NAME.VALUE);

  const setOriginalValue = (value: string) => setValue(FIELD_NAME.VALUE, value, { shouldValidate });
  const setExpectedValue = (value: string) => setValue(FIELD_NAME.EXPECTED_VALUE, value, { shouldValidate });

  const onValueChange = (value: string) => setExpectedValue(value);
  const onExpectedValueChange = (value: string) => setOriginalValue(value);

  const handleSubmit = form.handleSubmit((values) => {
    const onSuccess = () => {
      reset();
      alert.success('Successful transaction');
    };

    const onError = (error: WriteContractErrorType) => {
      const errorMessage = (error as BaseError).shortMessage || error.message;

      logger.error('Transfer Error', error);
      alert.error(errorMessage);
    };

    onSubmit(values).then(onSuccess).catch(onError);
  });

  useEffect(() => {
    reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);

  const setMaxBalance = () => {
    if (!ftBalance.formattedValue) throw new Error('Balance is not defined');

    setOriginalValue(ftBalance.formattedValue);
    onValueChange(ftBalance.formattedValue);
  };

  return {
    form,
    amount,
    onValueChange,
    onExpectedValueChange,
    handleSubmit,
    setMaxBalance,
  };
}

export { useSwapForm };