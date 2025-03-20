import { Outlet, ScrollRestoration } from 'react-router-dom';

import { ErrorBoundary, Footer, Header } from './components';
import { useAccountSync } from './features/wallet';
import { WithProviders } from './providers';

function Component() {
  useAccountSync();

  return (
    <>
      <Header />

      <main>
        <ErrorBoundary>
          <ScrollRestoration />

          <Outlet />
        </ErrorBoundary>
      </main>

      <Footer />
    </>
  );
}

const App = WithProviders(Component);

export { App };
