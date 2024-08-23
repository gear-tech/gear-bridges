import '@gear-js/vara-ui/dist/style.css';
import React from 'react';
import ReactDOM from 'react-dom/client';
import { RouterProvider, createBrowserRouter } from 'react-router-dom';

import { App } from './App';
import { ETH_CHAIN_ID, ETH_NODE_ADDRESS, NETWORK_NAME, ROUTE, SPEC, VARA_NODE_ADDRESS } from './consts';
import { NotFound, Home, Transactions, FAQ } from './pages';
import { logger } from './utils';
import './index.scss';

const logBridgeAddresses = () => {
  Object.entries(SPEC).forEach(([pair, bridge]) => {
    const capitalizedVaraNetworkName = NETWORK_NAME.VARA.charAt(0).toUpperCase() + NETWORK_NAME.VARA.slice(1);
    const capitalizedEthNetworkName = NETWORK_NAME.ETH.charAt(0).toUpperCase() + NETWORK_NAME.ETH.slice(1);

    logger.info(`${pair} ${capitalizedVaraNetworkName} bridge address`, bridge[NETWORK_NAME.VARA].address);
    logger.info(`${pair} ${capitalizedEthNetworkName} bridge address`, bridge[NETWORK_NAME.ETH].address);
  });
};

logger.info('Vara network address', VARA_NODE_ADDRESS);
logger.info('Eth network address', ETH_NODE_ADDRESS);
logger.info('Eth chain id', ETH_CHAIN_ID);
logBridgeAddresses();

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
