import { SPEC, NETWORK_NAME, FEE_DECIMALS, NETWORK_NATIVE_SYMBOL } from './spec';

const VARA_NODE_ADDRESS = import.meta.env.VITE_VARA_NODE_ADDRESS as string;
const ETH_NODE_ADDRESS = import.meta.env.VITE_ETH_NODE_ADDRESS as string;
const ETH_CHAIN_ID = Number(import.meta.env.VITE_ETH_CHAIN_ID as string);

const ROUTE = {
  HOME: '/',
  TRANSACTIONS: '/transactions',
  FAQ: '/faq',
};

export {
  VARA_NODE_ADDRESS,
  ETH_NODE_ADDRESS,
  ETH_CHAIN_ID,
  ROUTE,
  SPEC,
  NETWORK_NAME,
  FEE_DECIMALS,
  NETWORK_NATIVE_SYMBOL,
};