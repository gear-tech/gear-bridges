import { HexString } from '@gear-js/api';

import { DEFAULT_VALUES, FIELD_NAME } from '../consts';

type Values = typeof DEFAULT_VALUES;

type FormattedValues = {
  [FIELD_NAME.VALUE]: bigint;
  [FIELD_NAME.ADDRESS]: HexString;
};

export type { Values, FormattedValues };
