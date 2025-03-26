import { Controller, FieldError, get, useFormContext } from 'react-hook-form';
import { NumericFormat } from 'react-number-format';
import { parseUnits } from 'viem';

import { FormattedBalance } from '@/components';
import { cx } from '@/utils';

import { FIELD_NAME } from '../../consts';

import styles from './amount-input.module.scss';

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
            onValueChange={({ value }) => field.onChange(value)}
            allowNegative={false}
            aria-invalid={Boolean(error?.message)}
            thousandSeparator
            className={cx(styles.input, Number(fieldValue) && styles.active)}
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

function AmountInputValue({ decimals = 0 }: { decimals: number | undefined }) {
  const { watch } = useFormContext();
  const amount = (watch(FIELD_NAME.VALUE) as string) || '0';

  return (
    <FormattedBalance
      value={parseUnits(amount, decimals)}
      decimals={decimals}
      symbol=""
      className={cx(styles.amount, Number(amount) && styles.active)}
    />
  );
}

AmountInput.Error = AmountInputError;
AmountInput.Value = AmountInputValue;

export { AmountInput };
