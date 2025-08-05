import { Input as VaraInput } from '@gear-js/vara-ui';
import { useFormContext, get, FieldError } from 'react-hook-form';

import { InputProps } from '../types';

function Input({ name, onChange, ...props }: InputProps) {
  const { register, formState } = useFormContext();

  // use 'get' util as a safe way to access nested object properties:
  // https://github.com/react-hook-form/error-message/blob/2cb9e332bd4ca889ac028a423328e4b3db7d4765/src/ErrorMessage.tsx#L21
  const error = get(formState.errors, name) as FieldError | undefined;

  return <VaraInput {...props} {...register(name, { onChange })} error={error?.message} />;
}

export { Input };
