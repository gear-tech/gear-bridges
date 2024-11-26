import { InputProps as VaraInputProps } from '@gear-js/vara-ui';

type Props<T> = Omit<T, 'onBlur'> & {
  name: string;
};

type InputProps = Props<VaraInputProps>;

export type { Props, InputProps };
