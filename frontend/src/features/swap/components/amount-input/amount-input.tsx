import { Input } from '@gear-js/vara-ui';
import { Controller } from 'react-hook-form';
import { NumericFormat } from 'react-number-format';

import { FIELD_NAME } from '../../consts';

function AmountInput() {
  return (
    <Controller
      name={FIELD_NAME.VALUE}
      render={({ field, fieldState: { error } }) => (
        <NumericFormat
          value={field.value as string}
          onValueChange={({ value }) => field.onChange(value)}
          customInput={Input}
          label="Amount"
          error={error?.message}
          allowNegative={false}
          thousandSeparator
          block
        />
      )}
    />
  );
}

export { AmountInput };
