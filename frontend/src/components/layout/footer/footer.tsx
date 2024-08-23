import { Link } from 'react-router-dom';

import VaraLogoSVG from '@/assets/vara-logo.svg?react';
import { ROUTE } from '@/consts';

import { Container } from '../container';

import { LIST, SOCIALS } from './consts';
import styles from './footer.module.scss';

function Footer() {
  const currentYear = new Date().getFullYear();

  const renderLinks = (links: (typeof LIST)[number]['links']) =>
    links.map(({ text, href }) => (
      <li key={text} className={styles.link}>
        <a target="_blank" rel="noreferrer" href={href}>
          {text}
        </a>
      </li>
    ));

  const renderList = () =>
    LIST.map(({ heading, links }) => (
      <li key={heading}>
        <h3 className={styles.heading}>{heading}</h3>
        <ul>{renderLinks(links)}</ul>
      </li>
    ));

  const renderSocials = () =>
    SOCIALS.map(({ SVG, href }) => (
      <li key={href}>
        <a target="_blank" rel="noreferrer" href={href}>
          <SVG />
        </a>
      </li>
    ));

  return (
    <footer className={styles.footer}>
      <Container className={styles.container}>
        <div className={styles.listContainer}>
          <Link to={ROUTE.HOME}>
            <VaraLogoSVG width={140} height={90} />
          </Link>

          <ul className={styles.list}>{renderList()}</ul>
        </div>

        <div className={styles.copyrightContainer}>
          <small className={styles.copyright}>&copy; {currentYear} Gear Foundation, Inc. All Rights Reserved.</small>
          <ul className={styles.socials}>{renderSocials()}</ul>
        </div>
      </Container>
    </footer>
  );
}

export { Footer };
