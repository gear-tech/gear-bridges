import { Checkbox as VaraCheckbox, CheckboxProps } from '@gear-js/vara-ui';
import { useFormContext } from 'react-hook-form';

import { Props } from '../types';

function Checkbox({ name, ...props }: Props<CheckboxProps>) {
  const { register } = useFormContext();

  return <VaraCheckbox {...props} {...register(name)} />;
}

export { Checkbox };
