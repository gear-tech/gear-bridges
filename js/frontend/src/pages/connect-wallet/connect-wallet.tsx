import { Container } from '@/components';
import { NETWORK_TYPE, networkType } from '@/consts';
import { Wallet } from '@/features/wallet';

import styles from './connect-wallet.module.scss';

function ConnectWallet() {
  return (
    // currently only for My Tokens page, expand if there will be more private routes
    <Container className={styles.container}>
      <h1 className={styles.heading}>Vara Network Bridge</h1>
      <p className={styles.subheading}>
        Connect your wallet to {networkType === NETWORK_TYPE.TESTNET ? 'get test tokens' : 'see your tokens'}.
      </p>

      <Wallet />
    </Container>
  );
}

export { ConnectWallet };
