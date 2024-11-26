import { HexString } from '@gear-js/api';

const VARA_NODE_ADDRESS = import.meta.env.VITE_VARA_NODE_ADDRESS as string;
const ETH_NODE_ADDRESS = import.meta.env.VITE_ETH_NODE_ADDRESS as string;
const ETH_CHAIN_ID = Number(import.meta.env.VITE_ETH_CHAIN_ID as string);

const BRIDGING_PAYMENT_CONTRACT_ADDRESS = import.meta.env.VITE_BRIDGING_PAYMENT_CONTRACT_ADDRESS as HexString;

export { VARA_NODE_ADDRESS, ETH_NODE_ADDRESS, ETH_CHAIN_ID, BRIDGING_PAYMENT_CONTRACT_ADDRESS };
