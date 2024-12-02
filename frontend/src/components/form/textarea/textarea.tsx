import { Textarea as VaraTextarea, TextareaProps } from '@gear-js/vara-ui';
import { useFormContext } from 'react-hook-form';

import { Props } from '../types';

const Textarea = ({ name, ...props }: Props<TextareaProps>) => {
  const { register, formState } = useFormContext();
  const { errors } = formState;

  const error = errors[name]?.message?.toString();

  return <VaraTextarea {...props} {...register(name)} error={error} />;
};

export { Textarea };
