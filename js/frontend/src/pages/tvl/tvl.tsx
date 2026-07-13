import { Container } from '@/components';
import { TvlDashboard } from '@/features/tvl';

import styles from './tvl.module.scss';

function TVL() {
  return (
    <Container className={styles.container}>
      <TvlDashboard />
    </Container>
  );
}

export { TVL };
