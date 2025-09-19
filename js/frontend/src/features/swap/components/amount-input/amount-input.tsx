import { Controller, FieldError, get, useFormContext } from 'react-hook-form';
import { NumericFormat, numericFormatter } from 'react-number-format';
import { SourceType } from 'react-number-format/types/types';

import { TruncatedText } from '@/components';
import { cx } from '@/utils';

import { FIELD_NAME } from '../../consts';

import styles from './amount-input.module.scss';

const PROPS = {
  thousandSeparator: ' ',
  allowNegative: false,
} as const;

function AmountInput() {
  return (
    <Controller
      name={FIELD_NAME.VALUE}
      render={({ field, fieldState: { error } }) => {
        const fieldValue = field.value as string;

        return (
          <NumericFormat
            placeholder="0"
            value={fieldValue}
            onValueChange={({ value }, { source }) => {
              // cuz react-hook-form triggers onValueChange programmatically during form reset,
              // and with mode 'onChange' it results in immediate validation after
              if (source !== ('event' as SourceType)) return;

              field.onChange(value);
            }}
            aria-invalid={Boolean(error?.message)}
            className={cx(styles.input, Number(fieldValue) && styles.active)}
            {...PROPS}
          />
        );
      }}
    />
  );
}

function AmountInputError() {
  const { formState } = useFormContext();

  // use 'get' util as a safe way to access nested object properties:
  // https://github.com/react-hook-form/error-message/blob/2cb9e332bd4ca889ac028a423328e4b3db7d4765/src/ErrorMessage.tsx#L21
  const error = get(formState.errors, FIELD_NAME.VALUE) as FieldError | undefined;

  if (!error || !error.message) return;

  return <p className={styles.error}>{error.message}</p>;
}

function AmountInputValue() {
  const { watch } = useFormContext();
  const amount = (watch(FIELD_NAME.VALUE) as string) || '0';

  return (
    <TruncatedText
      value={numericFormatter(amount, PROPS)}
      className={cx(styles.amount, Number(amount) && styles.active)}
    />
  );
}

AmountInput.Error = AmountInputError;
AmountInput.Value = AmountInputValue;

export { AmountInput };
