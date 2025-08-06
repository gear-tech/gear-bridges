import { Container, LinkButton } from '@/components';
import { ROUTE } from '@/consts';

import styles from './not-found.module.scss';

function NotFound() {
  return (
    <Container className={styles.container}>
      <div>
        <h2>404</h2>
        <p className={styles.text}>Page not found</p>
      </div>

      <LinkButton to={ROUTE.HOME} text=" Back to Home" size="small" />
    </Container>
  );
}

export { NotFound };
