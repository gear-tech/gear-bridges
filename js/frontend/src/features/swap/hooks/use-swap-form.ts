import { useAlert, useAccount, useApi } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { formatUnits } from 'viem';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { usePendingTxsCount, useOptimisticPendingTxsCountUpdate } from '@/features/history/hooks';
import { useEthAccount } from '@/hooks';
import { isUndefined, logger, getErrorMessage } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { useBridgeContext } from '../context';
import { FormattedValues } from '../types';
import { getAmountSchema } from '../utils';

type Params = {
  accountBalance: bigint | undefined;
  ftBalance: bigint | undefined;
  shouldPayBridgingFee: boolean;
};

function useSwapForm({ accountBalance, ftBalance, shouldPayBridgingFee }: Params) {
  const { api } = useApi();
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const { token, network } = useBridgeContext();
  const alert = useAlert();

  const pendingTxsCount = usePendingTxsCount();
  const optimisticPendingTxsCountUpdate = useOptimisticPendingTxsCountUpdate();

  const valueSchema = getAmountSchema(
    token?.isNative,
    accountBalance,
    ftBalance,
    token?.decimals,
    network.isVara ? api?.existentialDeposit.toBigInt() : 0n,
  );

  const addressSchema = network.isVara ? ADDRESS_SCHEMA.ETH : ADDRESS_SCHEMA.VARA;

  const schema = z.object({
    [FIELD_NAME.VALUE]: valueSchema,
    [FIELD_NAME.ADDRESS]: addressSchema,
  });

  const form = useForm({
    defaultValues: DEFAULT_VALUES,
    resolver: zodResolver(schema),
    mode: 'onChange',
  });

  const { setValue, reset, formState } = form;
  const { isValid } = formState;

  const formValues = form.watch();
  const { amount } = formValues;
  const formattedValues = isValid ? schema.safeParse(formValues).data : undefined;

  const handleSubmit = (onSubmit: (values: FormattedValues) => Promise<unknown>) =>
    form.handleSubmit((values) => {
      const onSuccess = () => {
        reset();
        alert.success('Your transfer request was successful');

        // only for manual relay, failed vara fee tx case is not considered
        if (shouldPayBridgingFee) return;

        optimisticPendingTxsCountUpdate();

        // better to refetch after finalization, but it's hard with current send txs implementation
        setTimeout(() => void pendingTxsCount.refetch(), 5000);
      };

      const onError = (error: WriteContractErrorType | string) => {
        logger.error('Transfer Error', typeof error === 'string' ? new Error(error) : error);
        alert.error(getErrorMessage(error));
      };

      onSubmit(values).then(onSuccess).catch(onError);
    });

  const setMaxBalance = () => {
    const balance = token?.isNative ? accountBalance : ftBalance;

    if (isUndefined(token?.decimals)) throw new Error('Decimals are not defined');
    if (isUndefined(balance)) throw new Error('Balance is not defined');

    const formattedValue = formatUnits(balance, token.decimals);

    setValue(FIELD_NAME.VALUE, formattedValue, { shouldValidate: true });
  };

  useEffect(() => {
    form.clearErrors();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account, ethAccount.address]);

  return { form, amount, formattedValues, handleSubmit, setMaxBalance };
}

export { useSwapForm };
