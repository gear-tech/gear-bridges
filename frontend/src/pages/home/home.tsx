import { Container, LinkButton } from '@/components';
import { ROUTE } from '@/consts';
import { LatestTransactions } from '@/features/history';
import { Swap as SwapFeature } from '@/features/swap';
import { NetworkWalletField, SwapNetworkButton } from '@/features/wallet';

import styles from './home.module.scss';

function Home() {
  return (
    <Container.Live maxWidth="md" className={styles.container}>
      <SwapFeature
        renderWalletField={() => <NetworkWalletField />}
        renderSwapNetworkButton={(onClick: () => void) => <SwapNetworkButton onClick={onClick} />}
      />

      <div className={styles.transactions}>
        <header className={styles.header}>
          <h2 className={styles.heading}>Latest Transactions</h2>

          <LinkButton to={ROUTE.TRANSACTIONS} text="Show All" size="small" color="grey" />
        </header>

        <LatestTransactions />

        <LinkButton to={ROUTE.TRANSACTIONS} text="Load More" size="small" color="grey" block />
      </div>
    </Container.Live>
  );
}

export { Home };
