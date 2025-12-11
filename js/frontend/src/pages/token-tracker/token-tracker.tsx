import { Container } from '@/components';
import { TokensCard } from '@/features/token-tracker';

import styles from './token-tracker.module.scss';

function TokenTracker() {
  return (
    <Container className={styles.container}>
      <TokensCard.Vara />
      <TokensCard.Eth />
    </Container>
  );
}

export { TokenTracker };
