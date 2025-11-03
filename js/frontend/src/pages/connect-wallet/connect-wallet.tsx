import { Container } from '@/components';
import { useNetworkType } from '@/context/network-type';
import { Wallet } from '@/features/wallet';

import styles from './connect-wallet.module.scss';

function ConnectWallet() {
  const { isTestnet } = useNetworkType();

  return (
    // currently only for My Tokens page, expand if there will be more private routes
    <Container className={styles.container}>
      <h1 className={styles.heading}>Vara Network Bridge</h1>
      <p className={styles.subheading}>Connect your wallet to {isTestnet ? 'get test tokens' : 'see your tokens'}.</p>

      <Wallet />
    </Container>
  );
}

export { ConnectWallet };
