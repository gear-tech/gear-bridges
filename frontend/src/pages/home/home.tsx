import { Container } from '@/components';
import { PendingTransactionsWarning } from '@/features/history';
import { Swap } from '@/features/swap';

import styles from './home.module.scss';

function Home() {
  return (
    <Container maxWidth="640px" className={styles.container}>
      <PendingTransactionsWarning />
      <Swap />
    </Container>
  );
}

export { Home };
