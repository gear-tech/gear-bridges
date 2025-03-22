import { Button } from '@gear-js/vara-ui';
import { CSSProperties, useRef, useState, useEffect } from 'react';
import { Link, NavLink, useLocation } from 'react-router-dom';

import LogoSVG from '@/assets/logo.svg?react';
import { ROUTE } from '@/consts';
import { TransactionsCounter } from '@/features/history';

import { Container } from '../container';

import styles from './header.module.scss';

const LINKS = {
  [ROUTE.HOME]: 'Bridge',
  [ROUTE.TRANSACTIONS]: 'Transactions',
  [ROUTE.FAQ]: 'FAQ',
} as const;

function Header() {
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

    if (!linkRef) return setLinksStyle(undefined);

    setLinksStyle({
      '--active-link-width': `${linkRef.offsetWidth}px`,
      '--active-link-offset': `${linkRef.offsetLeft}px`,
    } as CSSProperties);
  }, [pathname]);

  const renderLinks = () =>
    Object.entries(LINKS).map(([to, text]) => {
      const setRef = (element: HTMLAnchorElement | null) => {
        linkRefs.current[to] = element;
      };

      return (
        <li key={to}>
          <NavLink to={to} className={styles.link} ref={setRef}>
            {text}
          </NavLink>
        </li>
      );
    });

  return (
    <header className={styles.header}>
      <Container className={styles.mainContainer}>
        <Link to={ROUTE.HOME}>
          <LogoSVG />
        </Link>

        <Button text="Connect Wallet" size="x-small" />
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
