import { Container } from '@/components';
import { Swap } from '@/features/swap';

function Home() {
  return (
    <Container maxWidth="640px">
      <Swap />
    </Container>
  );
}

export { Home };
