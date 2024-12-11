import { useAlert } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { logger } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { FormattedValues } from '../types';
import { getAmountSchema, getErrorMessage, getMergedBalance } from '../utils';

type Values = {
  value: bigint | undefined;
  formattedValue: string | undefined;
  isLoading: boolean;
};

function useSwapForm(
  isVaraNetwork: boolean,
  isNativeToken: boolean,
  accountBalance: Values,
  ftBalance: Values,
  decimals: number | undefined,
  fee: bigint | undefined,
  disabled: boolean,
  onSubmit: (values: FormattedValues) => Promise<unknown>,
  openTransactionModal: (amount: string, receiver: string) => void,
) {
  const alert = useAlert();

  const valueSchema = getAmountSchema(isNativeToken, accountBalance.value, ftBalance.value, fee, decimals);
  const addressSchema = isVaraNetwork ? ADDRESS_SCHEMA.ETH : ADDRESS_SCHEMA.VARA;

  const schema = z.object({
    [FIELD_NAME.VALUE]: valueSchema,
    [FIELD_NAME.ADDRESS]: addressSchema,
  });

  const form = useForm<typeof DEFAULT_VALUES, unknown, z.infer<typeof schema>>({
    defaultValues: DEFAULT_VALUES,
    resolver: zodResolver(schema),
  });

  const { setValue, reset, formState } = form;
  const amount = form.watch(FIELD_NAME.VALUE);

  const handleSubmit = form.handleSubmit((values) => {
    const onSuccess = () => {
      reset();
      alert.success('Transfer request is successful');
    };

    const onError = (error: WriteContractErrorType | string) => {
      logger.error('Transfer Error', typeof error === 'string' ? new Error(error) : error);
      alert.error(getErrorMessage(error));
    };

    openTransactionModal(values[FIELD_NAME.VALUE].toString(), values[FIELD_NAME.ADDRESS]);

    onSubmit(values).then(onSuccess).catch(onError);
  });

  useEffect(() => {
    reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);

  const setMaxBalance = () => {
    const balance = isNativeToken ? getMergedBalance(accountBalance, ftBalance, decimals) : ftBalance;
    if (!balance.formattedValue) throw new Error('Balance is not defined');

    const shouldValidate = formState.isSubmitted; // validating only if validation was already fired

    setValue(FIELD_NAME.VALUE, balance.formattedValue, { shouldValidate });
  };

  return { form, amount, handleSubmit, setMaxBalance };
}

export { useSwapForm };
