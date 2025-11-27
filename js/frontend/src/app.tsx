import { Outlet, ScrollRestoration } from 'react-router-dom';

import { ErrorBoundary, Header } from './components';
import { WithProviders } from './providers';

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
    </>
  );
}

const App = WithProviders(Component);

export { App };
