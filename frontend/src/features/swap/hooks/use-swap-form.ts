import { useAlert, useAccount, useApi } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { formatUnits } from 'viem';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { useEthAccount } from '@/hooks';
import { isUndefined, logger, getErrorMessage } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { useBridgeContext } from '../context';
import { FormattedValues } from '../types';
import { getAmountSchema } from '../utils';

type Values = {
  data: bigint | undefined;
  isLoading: boolean;
};

function useSwapForm(
  isVaraNetwork: boolean,
  accountBalance: Values,
  ftBalance: Values,
  decimals: number | undefined,
  onSubmit: (values: FormattedValues) => Promise<unknown>,
) {
  const { api } = useApi();
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const alert = useAlert();
  const { token } = useBridgeContext();

  const valueSchema = getAmountSchema(
    token?.isNative,
    accountBalance.data,
    ftBalance.data,
    decimals,
    isVaraNetwork ? api?.existentialDeposit.toBigInt() : 0n,
  );

  const addressSchema = isVaraNetwork ? ADDRESS_SCHEMA.ETH : ADDRESS_SCHEMA.VARA;

  const schema = z.object({
    [FIELD_NAME.VALUE]: valueSchema,
    [FIELD_NAME.ADDRESS]: addressSchema,
  });

  const form = useForm({
    defaultValues: DEFAULT_VALUES,
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
    const balance = token?.isNative ? accountBalance : ftBalance;
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
