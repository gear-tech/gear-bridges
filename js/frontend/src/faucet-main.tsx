import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Component, type ReactNode, StrictMode } from 'react';
import { createRoot } from 'react-dom/client';

import { Faucet } from './features/faucet';
import { deploymentOrigin } from './utils/deployment-origin';
import './faucet.css';

class FaucetBoundary extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() {
    return { failed: true };
  }
  render() {
    return this.state.failed ? (
      <p role="alert">Faucet configuration unavailable. No payout request can be submitted.</p>
    ) : (
      this.props.children
    );
  }
}

const root = createRoot(document.getElementById('root')!);
try {
  const origin = deploymentOrigin(import.meta.env.VITE_FAUCET_ORIGIN);
  if (window.location.origin !== origin || origin === deploymentOrigin(import.meta.env.VITE_FUNDS_ORIGIN))
    throw new Error('Faucet must be served on its separately approved origin');
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  root.render(
    <StrictMode>
      <FaucetBoundary>
        <QueryClientProvider client={client}>
          <Faucet />
        </QueryClientProvider>
      </FaucetBoundary>
    </StrictMode>,
  );
} catch {
  root.render(<p role="alert">Faucet disabled: this is not its approved separate HTTPS origin.</p>);
}
