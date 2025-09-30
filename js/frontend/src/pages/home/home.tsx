import { Container } from '@/components';
import { PendingTransactionsWarning } from '@/features/history';
import { Swap } from '@/features/swap';

import styles from './home.module.scss';

function Home() {
  return (
    <Container maxWidth="640px" className={styles.container}>
      <div className={styles.warning}>
        <div>
          <p className={styles.date}>⚠️ Testnet Update on 1 October.</p>

          <p>
            Bridge upgrade scheduled. Previous test transaction history will be removed from the UI. Real funds are not
            affected. New transactions will be visible as usual. Thanks for supporting the test phase!
          </p>
        </div>
      </div>

      <PendingTransactionsWarning />
      <Swap />
    </Container>
  );
}

export { Home };
