import { Container } from '@/components';
import { Swap } from '@/features/swap';

function Home() {
  return (
    <Container maxWidth="md">
      <Swap />
    </Container>
  );
}

export { Home };
