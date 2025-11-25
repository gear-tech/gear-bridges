import { ButtonProps, buttonStyles } from '@gear-js/vara-ui';
import { MouseEvent } from 'react';
import { Link } from 'react-router-dom';

import { cx } from '@/utils';

type Props = Pick<
  ButtonProps,
  'children' | 'color' | 'size' | 'disabled' | 'isLoading' | 'text' | 'icon' | 'block' | 'noWrap' | 'className'
> & {
  to: string;
  type?: 'internal' | 'external';
  onClick?: (event: MouseEvent) => void; // stop propagation for BlockNumberLink
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
  ...props // spreading props for tooltip to work
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
    <Link to={to} className={cn} {...props}>
      {Icon && <Icon className={buttonStyles.icon} />}
      {text && <span>{text}</span>}

      {children}
    </Link>
  ) : (
    <a href={to} target="_blank" rel="noreferrer" className={cn} {...props}>
      {Icon && <Icon className={buttonStyles.icon} />}
      {text && <span>{text}</span>}

      {children}
    </a>
  );
}

export { LinkButton };
