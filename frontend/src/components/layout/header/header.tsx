import { Link, NavLink } from 'react-router-dom';

import LogoSVG from '@/assets/logo.svg?react';
import { ROUTE } from '@/consts';
import { TransactionsCounter } from '@/features/history';

import { Container } from '../container';

import styles from './header.module.scss';

function Header() {
  return (
    <header className={styles.header}>
      <Container className={styles.container}>
        <TransactionsCounter />

        <Link to={ROUTE.HOME}>
          <LogoSVG />
        </Link>

        <nav className={styles.nav}>
          <NavLink to={ROUTE.HOME}>Home</NavLink>
          <NavLink to={ROUTE.TRANSACTIONS}>Transactions</NavLink>
          <NavLink to={ROUTE.FAQ}>FAQ</NavLink>
        </nav>
      </Container>
    </header>
  );
}

export { Header };
