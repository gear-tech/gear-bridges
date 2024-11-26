import { ButtonProps, buttonStyles } from '@gear-js/vara-ui';
import { Link } from 'react-router-dom';

import { cx } from '@/utils';

type Props = Omit<ButtonProps, 'onClick'> & {
  to: string;
};

function LinkButton({
  to,
  children,
  color = 'primary',
  size = 'default',
  disabled,
  isLoading,
  text,
  icon: Icon,
  block,
  noWrap,
  className,
}: Props) {
  return (
    <Link
      to={to}
      className={cx(
        buttonStyles.button,
        buttonStyles[color],
        color !== 'transparent' && buttonStyles[size],
        disabled && buttonStyles.disabled,
        isLoading && buttonStyles.loading,
        !text && buttonStyles.noText,
        block && buttonStyles.block,
        noWrap && buttonStyles.noWrap,
        className,
      )}>
      {Icon && <Icon />}
      {text && <span>{text}</span>}

      {children}
    </Link>
  );
}

export { LinkButton };
