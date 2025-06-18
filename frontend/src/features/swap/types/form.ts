import { HexString } from '@gear-js/api';

import { DEFAULT_VALUES, FIELD_NAME, SUBMIT_STATUS } from '../consts';

type Values = typeof DEFAULT_VALUES;

type FormattedValues = {
  [FIELD_NAME.VALUE]: bigint;
  [FIELD_NAME.ADDRESS]: HexString;
};

type SubmitStatus = (typeof SUBMIT_STATUS)[keyof typeof SUBMIT_STATUS];

type UseHandleSubmitParameters = {
  fee: bigint | undefined;
  allowance: bigint | undefined;
  accountBalance: bigint | undefined;
  onTransactionStart: (amount: bigint, receiver: string) => void;
};

type UseHandleSubmit = (params: UseHandleSubmitParameters) => {
  onSubmit: (values: FormattedValues) => Promise<unknown>;
  status: SubmitStatus;
  isPending: boolean;
  error: Error | null;
  isLoading?: boolean;
};

export type { Values, FormattedValues, UseHandleSubmitParameters, UseHandleSubmit };
