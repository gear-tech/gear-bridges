import '@gear-js/vara-ui/dist/style.css';
import * as Sentry from '@sentry/react';
import React from 'react';
import ReactDOM from 'react-dom/client';
import TagManager from 'react-gtm-module';
import { Outlet, RouterProvider, createBrowserRouter } from 'react-router-dom';

import { useAccountsConnection } from '@/hooks';

import { App } from './app';
import { ROUTE, GTM_ID, SENTRY_DSN } from './consts';
import { NotFound, Home, Transactions, FAQ, TokenTracker, ConnectWallet, Transaction } from './pages';

import './index.scss';

if (SENTRY_DSN)
  Sentry.init({
    dsn: SENTRY_DSN,
    integrations: [Sentry.replayIntegration()],
    replaysSessionSampleRate: 0,
    replaysOnErrorSampleRate: 1.0,
  });

if (GTM_ID) TagManager.initialize({ gtmId: GTM_ID });

// eslint-disable-next-line react-refresh/only-export-components
function PrivateRoute() {
  const { isAnyAccount, isAnyAccountLoading } = useAccountsConnection();

  // it's probably worth to check isConnecting too, but there is a bug:
  // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
  if (isAnyAccountLoading) return null;

  return isAnyAccount ? <Outlet /> : <ConnectWallet />;
}

const PUBLIC_ROUTES = [
  { path: ROUTE.HOME, element: <Home /> },
  { path: ROUTE.TRANSACTIONS, element: <Transactions /> },
  { path: ROUTE.FAQ, element: <FAQ /> },
  { path: ROUTE.TRANSACTION, element: <Transaction /> },
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
