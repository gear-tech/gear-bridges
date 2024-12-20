import { HexString } from '@gear-js/api';

import { FormattedValues } from './form';

type BalanceValues = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

type UseAccountBalance = () => {
  data: bigint | undefined;
  isLoading: boolean;
};

type UseFTBalance = (ftAddress: HexString | undefined) => {
  data: bigint | undefined;
  isLoading: boolean;
};

type UseFee = () => {
  fee: BalanceValues;
  isLoading: boolean;
};

type UseHandleSubmit = (
  ftAddress: HexString | undefined,
  feeValue: bigint | undefined,
  allowance: bigint | undefined,
  ftBalance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTxModal: (amount: string, receiver: string) => void,
) => Readonly<
  [
    {
      mutateAsync: (values: FormattedValues) => Promise<unknown>;
      reset: () => void;
      isPending: boolean;
      isSuccess: boolean;
      error: Error | null;
    },
    { isPending: boolean; isLoading?: boolean; error: Error | null },
    { isPending: boolean; error: Error | null }?,
  ]
>;

type UseFTAllowance = (address: HexString | undefined) => {
  data: bigint | undefined;
  isLoading: boolean;
  refetch: () => Promise<unknown>;
};

export type { UseAccountBalance, UseFTBalance, UseHandleSubmit, UseFee, UseFTAllowance };
