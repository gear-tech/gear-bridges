import { DEFAULT_VALUES, FIELD_NAME } from '../consts';

type Values = typeof DEFAULT_VALUES;

type FormattedValues = {
  [FIELD_NAME.VALUE]: bigint;
  [FIELD_NAME.EXPECTED_VALUE]: bigint;
  [FIELD_NAME.ADDRESS]: string;
};

export type { Values, FormattedValues };
