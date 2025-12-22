import { HexString } from '@gear-js/api';

import { FormattedValues, SubmitStatus } from './form';

type UseAccountBalance = () => {
  data: bigint | undefined;
  isLoading: boolean;
};

type UseFTBalance = (ftAddress: HexString | undefined) => {
  data: bigint | undefined;
  isLoading: boolean;
};

type UseFee = () => {
  bridgingFee: bigint | undefined;
  isLoading: boolean;
  vftManagerFee?: bigint | undefined;
  priorityFee?: bigint | undefined;
};

type UseSendTxs = (params: {
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  priorityFee: bigint | undefined;
  shouldPayPriorityFee: boolean;
  vftManagerFee: bigint | undefined;
  ftBalance: bigint | undefined;
  onTransactionStart: (values: FormattedValues) => void;
}) => {
  mutateAsync: (values: FormattedValues) => Promise<unknown>;
  status: SubmitStatus;
  isPending: boolean;
  error: Error | null;
};

type UseTxsEstimate = (params: {
  formValues: FormattedValues | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  priorityFee: bigint | undefined;
  shouldPayPriorityFee: boolean;
  vftManagerFee: bigint | undefined;
  ftBalance: bigint | undefined;
}) => {
  data: { requiredBalance: bigint; fees: bigint } | undefined;
  isLoading: boolean;
};

export type { UseAccountBalance, UseFTBalance, UseFee, UseSendTxs, UseTxsEstimate };
