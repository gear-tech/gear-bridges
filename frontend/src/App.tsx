import { Outlet, ScrollRestoration } from 'react-router-dom';

import { ErrorBoundary, Footer, Header } from './components';
import { useAccountSync } from './features/wallet';
import { withProviders } from './providers';

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

const App = withProviders(Component);

export { App };
