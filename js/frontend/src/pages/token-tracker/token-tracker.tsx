import { Container } from '@/components';
import { TokensCard } from '@/features/token-tracker';

function TokenTracker() {
  return (
    <Container maxWidth="490px">
      <TokensCard />
    </Container>
  );
}

export { TokenTracker };
