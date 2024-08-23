import { Radio as VaraRadio, RadioProps } from '@gear-js/vara-ui';
import { useFormContext } from 'react-hook-form';

import { Props } from '../types';

function Radio({ name, ...props }: Props<RadioProps>) {
  const { register } = useFormContext();

  return <VaraRadio {...props} {...register(name)} />;
}

export { Radio };
