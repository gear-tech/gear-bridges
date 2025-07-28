import WarningSVG from '@/assets/warning.svg?react';
import { Container } from '@/components';
import { PendingTransactionsWarning } from '@/features/history';
import { Swap } from '@/features/swap';

import styles from './home.module.scss';

function Home() {
  return (
    <Container maxWidth="640px" className={styles.container}>
      <div className={styles.warning}>
        <WarningSVG className={styles.icon} />

        <div className={styles.text}>
          <p className={styles.heading}>Testnet Update on 31-Jul-2025.</p>
          <p>
            A major Bridge upgrade is scheduled, which includes important fixes, stability improvements, and a
            transition of the Ethereum testnet environment from Holesky to Hoodi.
          </p>
          <p>
            Please note - previous test transaction history will be removed from the UI (real funds are not affected).
            New transactions will appear as usual.
          </p>
          <p>Thanks for supporting the test phase!</p>
        </div>
      </div>

      <PendingTransactionsWarning />
      <Swap />
    </Container>
  );
}

export { Home };
