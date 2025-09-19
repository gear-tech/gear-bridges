import { HexString } from '@gear-js/api';

import { DEFAULT_VALUES, FIELD_NAME, SUBMIT_STATUS } from '../consts';

type Values = typeof DEFAULT_VALUES;

type FormattedValues = {
  [FIELD_NAME.VALUE]: bigint;
  [FIELD_NAME.ADDRESS]: HexString;
};

type SubmitStatus = (typeof SUBMIT_STATUS)[keyof typeof SUBMIT_STATUS];

type UseHandleSubmitParameters = {
  formValues: FormattedValues | undefined;
  bridgingFee: bigint | undefined;
  shouldPayBridgingFee: boolean;
  vftManagerFee: bigint | undefined;
  onTransactionStart: (values: FormattedValues) => void;
};

type UseHandleSubmit = (params: UseHandleSubmitParameters) => {
  mutateAsync: (values: FormattedValues) => Promise<unknown>;
  status: SubmitStatus;
  isPending: boolean;
  error: Error | null;
  txsEstimate: { requiredBalance: bigint; fees: bigint } | undefined;
};

export type { Values, FormattedValues, UseHandleSubmitParameters, UseHandleSubmit };
