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

type UseHandleSubmitParameters = {
  fee: bigint | undefined;
  allowance: bigint | undefined;
  accountBalance: bigint | undefined;
  onTransactionStart: (amount: bigint, receiver: string) => void;
};

type UseHandleSubmit = (params: UseHandleSubmitParameters) => {
  onSubmit: (values: FormattedValues) => Promise<unknown>;
  status: 'success' | 'bridge' | 'fee' | 'mint' | 'approve' | 'permit';
  isPending: boolean;
  error: Error | null;
  isLoading?: boolean;
};

type UseFTAllowance = (address: HexString | undefined) => {
  data: bigint | undefined;
  isLoading: boolean;
  refetch: () => Promise<unknown>;
};

export type { UseAccountBalance, UseFTBalance, UseHandleSubmitParameters, UseHandleSubmit, UseFee, UseFTAllowance };
