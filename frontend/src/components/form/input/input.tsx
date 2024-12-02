import { Input as VaraInput } from '@gear-js/vara-ui';
import { useFormContext } from 'react-hook-form';

import { InputProps } from '../types';

function Input({ name, onChange, ...props }: InputProps) {
  const { register, formState } = useFormContext();
  const { errors } = formState;

  const error = errors[name]?.message?.toString();

  return <VaraInput {...props} {...register(name, { onChange })} error={error} />;
}

export { Input };
