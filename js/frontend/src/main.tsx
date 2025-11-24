import '@gear-js/vara-ui/dist/style.css';
import { useAccount } from '@gear-js/react-hooks';
import React from 'react';
import ReactDOM from 'react-dom/client';
import TagManager from 'react-gtm-module';
import { Outlet, RouterProvider, createBrowserRouter } from 'react-router-dom';

import { App } from './app';
import { ROUTE, GTM_ID } from './consts';
import { useEthAccount } from './hooks';
import { NotFound, Home, Transactions, FAQ, TokenTracker, ConnectWallet, Transaction } from './pages';

import './index.scss';

if (GTM_ID) TagManager.initialize({ gtmId: GTM_ID });

// eslint-disable-next-line react-refresh/only-export-components
function PrivateRoute() {
  const { account, isAccountReady } = useAccount();
  const ethAccount = useEthAccount();

  // it's probably worth to check isConnecting too, but there is a bug:
  // no extensions -> open any wallet's QR code -> close modal -> isConnecting is still true
  if (!isAccountReady || ethAccount.isReconnecting) return null;

  return account || ethAccount.address ? <Outlet /> : <ConnectWallet />;
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
