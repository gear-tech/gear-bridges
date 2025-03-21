import '@gear-js/vara-ui/dist/style.css';
import React from 'react';
import ReactDOM from 'react-dom/client';
import { RouterProvider, createBrowserRouter } from 'react-router-dom';

import { App } from './app';
import { ETH_CHAIN_ID, ETH_NODE_ADDRESS, ROUTE, VARA_NODE_ADDRESS } from './consts';
import { NotFound, Home, Transactions, FAQ } from './pages';
import { logger } from './utils';
import './index.scss';

logger.info('Vara network address', VARA_NODE_ADDRESS);
logger.info('Eth network address', ETH_NODE_ADDRESS);
logger.info('Eth chain id', ETH_CHAIN_ID);

const ROUTES = [
  { path: ROUTE.HOME, element: <Home /> },
  { path: ROUTE.TRANSACTIONS, element: <Transactions /> },
  { path: ROUTE.FAQ, element: <FAQ /> },
  { path: '*', element: <NotFound /> },
];

const router = createBrowserRouter([{ element: <App />, children: ROUTES }]);

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <RouterProvider router={router} />
  </React.StrictMode>,
);
