import { Outlet, ScrollRestoration } from 'react-router-dom';

import { ErrorBoundary, Footer, Header } from './components';
import { withProviders } from './providers';

function Component() {
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
