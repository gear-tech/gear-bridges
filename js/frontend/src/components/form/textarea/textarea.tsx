import { Textarea as VaraTextarea, TextareaProps } from '@gear-js/vara-ui';
import { FieldError, get, useFormContext } from 'react-hook-form';

import { Props } from '../types';

const Textarea = ({ name, ...props }: Props<TextareaProps>) => {
  const { register, formState } = useFormContext();

  // use 'get' util as a safe way to access nested object properties:
  // https://github.com/react-hook-form/error-message/blob/2cb9e332bd4ca889ac028a423328e4b3db7d4765/src/ErrorMessage.tsx#L21
  const error = get(formState.errors, name) as FieldError | undefined;

  return <VaraTextarea {...props} {...register(name)} error={error?.message} />;
};

export { Textarea };
