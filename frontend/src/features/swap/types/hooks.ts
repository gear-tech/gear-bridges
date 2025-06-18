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
  feeValue: bigint | undefined,
  allowance: bigint | undefined,
  accountBalance: bigint | undefined,
  openTxModal: (amount: string, receiver: string) => void,
) => {
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

export type { UseAccountBalance, UseFTBalance, UseHandleSubmit, UseFee, UseFTAllowance };
