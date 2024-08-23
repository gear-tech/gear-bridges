import { Select as VaraSelect, SelectProps } from '@gear-js/vara-ui';
import { useFormContext } from 'react-hook-form';

import { Props } from '../types';

function Select({ name, onChange, ...props }: Props<SelectProps>) {
  const { register } = useFormContext();

  return <VaraSelect {...props} {...register(name, { onChange })} />;
}

export { Select };
