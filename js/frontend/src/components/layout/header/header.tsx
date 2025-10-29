import { useAccount } from '@gear-js/react-hooks';
import { CSSProperties, useRef, useState, useEffect } from 'react';
import { Link, NavLink, useLocation } from 'react-router-dom';

import LogoSVG from '@/assets/logo.svg?react';
import { ROUTE } from '@/consts';
import { TransactionsCounter } from '@/features/history';
import { NetworkSwitch } from '@/features/network-switch';
import { LockedBalanceTooltip } from '@/features/token-tracker';
import { Wallet } from '@/features/wallet';
import { useEthAccount } from '@/hooks';

import { Container } from '../container';

import { LINKS } from './consts';
import styles from './header.module.scss';

function Header() {
  const { account } = useAccount();
  const ethAccount = useEthAccount();
  const isAnyAccount = account || ethAccount.isConnected;

  const { pathname } = useLocation();
  const [linksStyle, setLinksStyle] = useState<CSSProperties>();

  const linksRef = useRef<HTMLUListElement>(null);
  const linkRefs = useRef<{ [key: string]: HTMLAnchorElement | null }>({
    [ROUTE.HOME]: null,
    [ROUTE.TRANSACTIONS]: null,
    [ROUTE.FAQ]: null,
  });

  useEffect(() => {
    const linkRef = linkRefs.current[pathname];

    if (!linksRef.current || !linkRef) return setLinksStyle(undefined);

    const linksRect = linksRef.current.getBoundingClientRect();
    const linkRect = linkRef.getBoundingClientRect();
    const offset = linkRect.left - linksRect.left;

    setLinksStyle({
      '--active-link-width': `${linkRect.width}px`,
      '--active-link-offset': `${offset}px`,
    } as CSSProperties);
  }, [pathname, isAnyAccount]);

  const renderLinks = () =>
    Object.entries(LINKS).map(([to, text]) => {
      const isTokensLink = to === ROUTE.TOKEN_TRACKER;

      if (isTokensLink && !isAnyAccount) return;

      const setRef = (element: HTMLAnchorElement | null) => {
        linkRefs.current[to] = element;
      };

      return (
        <li key={to}>
          <NavLink to={to} className={styles.link} ref={setRef}>
            {text}
          </NavLink>

          {isTokensLink && <LockedBalanceTooltip />}
        </li>
      );
    });

  return (
    <header className={styles.header}>
      <Container className={styles.mainContainer}>
        <div className={styles.logoContainer}>
          <Link to={ROUTE.HOME} className={styles.logo}>
            <LogoSVG />
          </Link>

          <NetworkSwitch />
        </div>

        <Wallet />
      </Container>

      <nav className={styles.nav}>
        <Container className={styles.navContainer}>
          <ul className={styles.links} ref={linksRef} style={linksStyle}>
            {renderLinks()}
          </ul>

          <TransactionsCounter />
        </Container>
      </nav>
    </header>
  );
}

export { Header };
