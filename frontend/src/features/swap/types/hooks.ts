import { HexString } from '@gear-js/api';

import { FormattedValues } from './form';

type BalanceValues = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

type UseBalance = (ftAddress: HexString | undefined) => BalanceValues & {
  decimals: number | undefined;
  isLoading: boolean;
};

type UseHandleSubmit = (
  ftAddress: HexString | undefined,
  feeValue?: bigint | undefined,
) => {
  onSubmit: (values: FormattedValues, reset: () => void) => void;
  isSubmitting: boolean;
};

export type { UseBalance, UseHandleSubmit };
