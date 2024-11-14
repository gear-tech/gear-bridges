import { HexString } from '@gear-js/api';

import { FormattedValues } from './form';

type BalanceValues = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

type UseAccountBalance = () => BalanceValues & {
  isLoading: boolean;
};

type UseFTBalance = (ftAddress: HexString | undefined) => BalanceValues & {
  decimals: number | undefined;
  isLoading: boolean;
};

type UseFee = () => {
  fee: BalanceValues;
  isLoading: boolean;
};

type UseHandleSubmit = (
  ftAddress: HexString | undefined,
  feeValue?: bigint | undefined,
) => {
  onSubmit: (values: FormattedValues, reset: () => void) => void;
  isSubmitting: boolean;
  isLoading?: boolean;
};

export type { UseAccountBalance, UseFTBalance, UseHandleSubmit, UseFee };
