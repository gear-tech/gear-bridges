import '@gear-js/vara-ui/dist/style.css';
import { useAccount } from '@gear-js/react-hooks';
import React from 'react';
import ReactDOM from 'react-dom/client';
import { Navigate, Outlet, RouterProvider, createBrowserRouter } from 'react-router-dom';

import { App } from './app';
import { ETH_CHAIN_ID, ETH_NODE_ADDRESS, ROUTE, VARA_NODE_ADDRESS } from './consts';
import { useEthAccount } from './hooks';
import { NotFound, Home, Transactions, FAQ, TokenTracker } from './pages';
import { logger } from './utils';

import './index.scss';

logger.info('Vara network address', VARA_NODE_ADDRESS);
logger.info('Eth network address', ETH_NODE_ADDRESS);
logger.info('Eth chain id', ETH_CHAIN_ID);

// eslint-disable-next-line react-refresh/only-export-components
function PrivateRoute() {
  const { account, isAccountReady } = useAccount();
  const ethAccount = useEthAccount();

  // it's probably worth to check isConnecting too, but there is a bug:
  // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
  if (!isAccountReady || ethAccount.isReconnecting) return null;

  return account || ethAccount.address ? <Outlet /> : <Navigate to={ROUTE.HOME} />;
}

const PUBLIC_ROUTES = [
  { path: ROUTE.HOME, element: <Home /> },
  { path: ROUTE.TRANSACTIONS, element: <Transactions /> },
  { path: ROUTE.FAQ, element: <FAQ /> },
  { path: '*', element: <NotFound /> },
];

const PRIVATE_ROUTES = [{ path: ROUTE.TOKEN_TRACKER, element: <TokenTracker /> }];

const ROUTES = [
  ...PUBLIC_ROUTES,

  {
    element: <PrivateRoute />,
    children: PRIVATE_ROUTES,
  },
];

const router = createBrowserRouter([{ element: <App />, children: ROUTES }]);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>,
);
