import { NavigationMenu } from '@base-ui-components/react';
import { useAccount } from '@gear-js/react-hooks';
import { Link, useLocation } from 'react-router-dom';

import { Card } from '@/components/card';
import { ROUTE } from '@/consts';
import { LockedBalanceTooltip } from '@/features/token-tracker';
import { useEthAccount } from '@/hooks';
import { cx } from '@/utils';

import { LINKS } from './consts';
import DotsSVG from './dots.svg?react';
import styles from './mobile-menu.module.scss';

function MobileMenu() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isAnyAccount = account || ethAccount.isConnected;

  const { pathname } = useLocation();

  const renderLinks = () =>
    Object.entries(LINKS).map(([to, text]) => {
      const isActive = pathname === to;

      const isTokensLink = to === ROUTE.TOKEN_TRACKER;

      if (isTokensLink && !isAnyAccount) return;

      return (
        <li key={to}>
          <NavigationMenu.Link
            render={(props) => (
              <Link to={to} {...props} className={cx(styles.link, isActive && styles.active)}>
                <span>{text}</span>

                {isTokensLink && <LockedBalanceTooltip />}
              </Link>
            )}
          />
        </li>
      );
    });

  return (
    <NavigationMenu.Root className={styles.root}>
      <NavigationMenu.List>
        <NavigationMenu.Item>
          <NavigationMenu.Trigger className={styles.trigger}>
            <span>{LINKS[pathname]}</span>

            <NavigationMenu.Icon>
              <DotsSVG />
            </NavigationMenu.Icon>
          </NavigationMenu.Trigger>

          <NavigationMenu.Content render={Card} className={styles.content}>
            <ul>{renderLinks()}</ul>
          </NavigationMenu.Content>
        </NavigationMenu.Item>
      </NavigationMenu.List>

      <NavigationMenu.Portal>
        <NavigationMenu.Positioner collisionPadding={{ top: 8, bottom: 8, left: 16, right: 16 }}>
          <NavigationMenu.Popup className={styles.popup}>
            <NavigationMenu.Viewport />
          </NavigationMenu.Popup>
        </NavigationMenu.Positioner>
      </NavigationMenu.Portal>
    </NavigationMenu.Root>
  );
}

export { MobileMenu };
