import { useAlert, useAccount } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { formatUnits } from 'viem';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { useEthAccount } from '@/hooks';
import { isUndefined, logger, getErrorMessage } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { FormattedValues } from '../types';
import { getAmountSchema, getMergedBalance } from '../utils';

type Values = {
  data: bigint | undefined;
  isLoading: boolean;
};

function useSwapForm(
  isVaraNetwork: boolean,
  isNativeToken: boolean,
  accountBalance: Values,
  ftBalance: Values,
  decimals: number | undefined,
  onSubmit: (values: FormattedValues) => Promise<unknown>,
) {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const alert = useAlert();

  const valueSchema = getAmountSchema(isNativeToken, accountBalance.data, ftBalance.data, decimals);
  const addressSchema = isVaraNetwork ? ADDRESS_SCHEMA.ETH : ADDRESS_SCHEMA.VARA;

  const schema = z.object({
    [FIELD_NAME.VALUE]: valueSchema,
    [FIELD_NAME.ADDRESS]: addressSchema,
  });

  const form = useForm<typeof DEFAULT_VALUES, unknown, z.infer<typeof schema>>({
    defaultValues: DEFAULT_VALUES,

    // @ts-expect-error -- revisit after next pr are released:
    // https://github.com/react-hook-form/react-hook-form/pull/12638
    // https://github.com/react-hook-form/resolvers/pull/753
    resolver: zodResolver(schema),
  });

  const { setValue, reset, formState } = form;
  const amount = form.watch(FIELD_NAME.VALUE);

  const handleSubmit = form.handleSubmit((values) => {
    const onSuccess = () => {
      reset();
      alert.success('Your transfer request was successful');
    };

    const onError = (error: WriteContractErrorType | string) => {
      logger.error('Transfer Error', typeof error === 'string' ? new Error(error) : error);
      alert.error(getErrorMessage(error));
    };

    onSubmit(values).then(onSuccess).catch(onError);
  });

  const setMaxBalance = () => {
    const balance = isNativeToken ? getMergedBalance(accountBalance, ftBalance) : ftBalance;
    if (isUndefined(decimals)) throw new Error('Decimals are not defined');
    if (isUndefined(balance.data)) throw new Error('Balance is not defined');

    const formattedValue = formatUnits(balance.data, decimals);
    const shouldValidate = formState.isSubmitted; // validating only if validation was already fired

    setValue(FIELD_NAME.VALUE, formattedValue, { shouldValidate });
  };

  useEffect(() => {
    form.clearErrors();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account, ethAccount.address]);

  return { form, amount, handleSubmit, setMaxBalance };
}

export { useSwapForm };
