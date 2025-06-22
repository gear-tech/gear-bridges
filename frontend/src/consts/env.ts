const VARA_NODE_ADDRESS = import.meta.env.VITE_VARA_NODE_ADDRESS as string;
const ETH_NODE_ADDRESS = import.meta.env.VITE_ETH_NODE_ADDRESS as string;
const ETH_CHAIN_ID = Number(import.meta.env.VITE_ETH_CHAIN_ID as string);

const GTM_ID = import.meta.env.VITE_GTM_ID as string | undefined;

export { VARA_NODE_ADDRESS, ETH_NODE_ADDRESS, ETH_CHAIN_ID, GTM_ID };
