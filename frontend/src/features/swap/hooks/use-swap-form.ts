import { useAlert, useAccount, useApi } from '@gear-js/react-hooks';
import { zodResolver } from '@hookform/resolvers/zod';
import { UseMutationResult } from '@tanstack/react-query';
import { useEffect } from 'react';
import { useForm } from 'react-hook-form';
import { formatUnits } from 'viem';
import { WriteContractErrorType } from 'wagmi/actions';
import { z } from 'zod';

import { useEthAccount, useVaraSymbol } from '@/hooks';
import { isUndefined, logger, getErrorMessage, definedAssert } from '@/utils';

import { FIELD_NAME, DEFAULT_VALUES, ADDRESS_SCHEMA } from '../consts';
import { useBridgeContext } from '../context';
import { InsufficientAccountBalanceError } from '../errors';
import { FormattedValues } from '../types';
import { getAmountSchema } from '../utils';

type Params = {
  accountBalance: bigint | undefined;
  ftBalance: bigint | undefined;
  requiredBalance: UseMutationResult<bigint, Error, FormattedValues, unknown>;
  onSubmit: (values: FormattedValues) => Promise<unknown>;
  onValidation: () => void;
};

function useSwapForm({ accountBalance, ftBalance, onSubmit, requiredBalance, onValidation }: Params) {
  const { api } = useApi();
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const { token, network } = useBridgeContext();
  const varaSymbol = useVaraSymbol();
  const alert = useAlert();

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
  });

  const { setValue, reset, formState } = form;
  const amount = form.watch(FIELD_NAME.VALUE);

  const validateBalance = async (values: FormattedValues) => {
    definedAssert(accountBalance, 'Account balance is not defined');
    definedAssert(varaSymbol, 'Vara symbol is not defined');

    const _requiredBalance = await requiredBalance.mutateAsync(values);
    const symbol = network.isVara ? varaSymbol : 'ETH';

    if (accountBalance < _requiredBalance) throw new InsufficientAccountBalanceError(symbol, _requiredBalance);
  };

  const handleSubmit = form.handleSubmit((values) => {
    const onSuccess = () => {
      reset();
      requiredBalance.reset();

      alert.success('Your transfer request was successful');
    };

    const onError = (error: WriteContractErrorType | string) => {
      logger.error('Transfer Error', typeof error === 'string' ? new Error(error) : error);
      alert.error(getErrorMessage(error));
    };

    if (isUndefined(requiredBalance.data)) return validateBalance(values).then(onValidation).catch(onError);

    onSubmit(values).then(onSuccess).catch(onError);
  });

  const setMaxBalance = () => {
    const balance = token?.isNative ? accountBalance : ftBalance;

    if (isUndefined(token?.decimals)) throw new Error('Decimals are not defined');
    if (isUndefined(balance)) throw new Error('Balance is not defined');

    const formattedValue = formatUnits(balance, token.decimals);
    const shouldValidate = formState.isSubmitted; // validating only if validation was already fired

    setValue(FIELD_NAME.VALUE, formattedValue, { shouldValidate });
  };

  useEffect(() => {
    form.clearErrors();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [account, ethAccount.address]);

  useEffect(() => {
    requiredBalance.reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [amount, token?.address]);

  return { form, amount, handleSubmit, setMaxBalance };
}

export { useSwapForm };
