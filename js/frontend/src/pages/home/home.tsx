import { Container } from '@/components';
import { useNetworkType } from '@/context/network-type';
import { PendingTransactionsWarning } from '@/features/history';
import { Swap } from '@/features/swap';

import styles from './home.module.scss';

function Home() {
  const { NETWORK_PRESET } = useNetworkType();
  return (
    <Container maxWidth="640px" className={styles.container}>
      {NETWORK_PRESET.INBOUND_PROOF_PROFILE.hold && (
        <p role="status">
          {NETWORK_PRESET.INBOUND_PROOF_PROFILE.hold} Ethereum → Vara claims remain pending; no finalized completion is
          asserted.
        </p>
      )}
      <PendingTransactionsWarning />
      <Swap />
    </Container>
  );
}

export { Home };
