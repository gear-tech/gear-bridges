import { HexString } from '@gear-js/api';

import { DEFAULT_VALUES, FIELD_NAME, SUBMIT_STATUS } from '../consts';

type Values = typeof DEFAULT_VALUES;

type FormattedValues = {
  [FIELD_NAME.VALUE]: bigint;
  [FIELD_NAME.ADDRESS]: HexString;
};

type SubmitStatus = (typeof SUBMIT_STATUS)[keyof typeof SUBMIT_STATUS];

export type { Values, FormattedValues, SubmitStatus };
