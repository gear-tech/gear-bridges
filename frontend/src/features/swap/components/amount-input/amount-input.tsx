import { Input } from '@gear-js/vara-ui';
import { Controller } from 'react-hook-form';
import { NumericFormat } from 'react-number-format';
import { SourceType } from 'react-number-format/types/types';

import { FIELD_NAME } from '../../consts';

type Props = {
  onChange: (value: string) => void;
};

function AmountInput({ onChange }: Props) {
  return (
    <Controller
      name={FIELD_NAME.VALUE}
      render={({ field, fieldState: { error } }) => (
        <NumericFormat
          value={field.value as string}
          onValueChange={({ value }, { source }) => {
            // seems like onValueChange is triggered when input value formatting changes too,
            // therefore resulting in twice onChange calls in our case of two inputs that depend on each other.
            // skipping this behaviour to rely only on user's input.
            if (source === ('prop' as SourceType)) return; // assertion cuz of enum import error

            field.onChange(value);
            onChange(value);
          }}
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
