import { useAlert } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { formatUnits, parseUnits } from 'viem';
import { BaseError } from 'wagmi';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { isUndefined, logger } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { FormattedValues } from '../types';
import { getAmountSchema, getMergedBalance } from '../utils';

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
  const expectedValueSchema = getAmountSchema(isNativeToken, accountBalance.value, ftBalance.value, 0n, decimals);
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

  const getValueWithFee = (value: string, operator: '+' | '-' = '+') => {
    if (isUndefined(fee)) throw new Error('Fee is not defined');
    if (isUndefined(decimals)) throw new Error('Decimals is not defined');
    if (!value) return value;

    const chainValue = parseUnits(value, decimals);
    const valueWithFee = operator === '+' ? chainValue + fee : chainValue - fee;

    return valueWithFee < 0 ? '0' : formatUnits(valueWithFee, decimals);
  };

  const onValueChange = (value: string) => setExpectedValue(isNativeToken ? getValueWithFee(value, '-') : value);
  const onExpectedValueChange = (value: string) => setOriginalValue(isNativeToken ? getValueWithFee(value) : value);

  const handleSubmit = form.handleSubmit((values) => {
    const onSuccess = () => {
      reset();
      alert.success('Successful transaction');
    };

    // string is only for cancelled sign and send popup error during useSendProgramTransaction
    // reevaluate after @gear-js/react-hooks update
    const onError = (error: WriteContractErrorType | string) => {
      logger.error('Transfer Error', typeof error === 'string' ? new Error(error) : error);
      alert.error(typeof error === 'string' ? error : (error as BaseError).shortMessage || error.message);
    };

    openTransactionModal(values[FIELD_NAME.EXPECTED_VALUE].toString(), values[FIELD_NAME.ADDRESS]);

    onSubmit(values).then(onSuccess).catch(onError);
  });

  useEffect(() => {
    reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [disabled]);

  const setMaxBalance = () => {
    const balance = isNativeToken ? getMergedBalance(accountBalance, ftBalance, decimals) : ftBalance;
    if (!balance.formattedValue) throw new Error('Balance is not defined');

    setOriginalValue(balance.formattedValue);
    onValueChange(balance.formattedValue);
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
