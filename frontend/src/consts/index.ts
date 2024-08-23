import { decodeAddress } from '@gear-js/api';
import { z } from 'zod';

import { isValidAddress } from '../utils';

import { SPEC, NETWORK_NAME, NETWORK_NATIVE_SYMBOL } from './spec';

const VARA_NODE_ADDRESS = import.meta.env.VITE_VARA_NODE_ADDRESS as string;
const ETH_NODE_ADDRESS = import.meta.env.VITE_ETH_NODE_ADDRESS as string;
const ETH_CHAIN_ID = Number(import.meta.env.VITE_ETH_CHAIN_ID as string);

const ROUTE = {
  HOME: '/',
  TRANSACTIONS: '/transactions',
  FAQ: '/faq',
};

const SCHEMA = {
  ADDRESS: z
    .string()
    .trim()
    .refine((value) => isValidAddress(value), 'Invalid address')
    .transform((value) => decodeAddress(value)),
};

export { VARA_NODE_ADDRESS, ETH_NODE_ADDRESS, ETH_CHAIN_ID, ROUTE, SCHEMA, SPEC, NETWORK_NAME, NETWORK_NATIVE_SYMBOL };
