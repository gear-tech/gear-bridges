import { ButtonProps, buttonStyles } from '@gear-js/vara-ui';
import { Link } from 'react-router-dom';

import { cx } from '@/utils';

type Props = Omit<ButtonProps, 'onClick' | 'type'> & {
  to: string;
  type?: 'internal' | 'external';
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
  type = 'internal',
}: Props) {
  const cn = cx(
    'linkButton',
    buttonStyles.button,
    buttonStyles[color],
    color !== 'transparent' && buttonStyles[size],
    disabled && buttonStyles.disabled,
    isLoading && buttonStyles.loading,
    !text && buttonStyles.noText,
    block && buttonStyles.block,
    noWrap && buttonStyles.noWrap,
    className,
  );

  return type === 'internal' ? (
    <Link to={to} className={cn}>
      {Icon && <Icon className={buttonStyles.icon} />}
      {text && <span>{text}</span>}

      {children}
    </Link>
  ) : (
    <a href={to} target="_blank" rel="noreferrer" className={cn}>
      {Icon && <Icon className={buttonStyles.icon} />}
      {text && <span>{text}</span>}

      {children}
    </a>
  );
}

export { LinkButton };
