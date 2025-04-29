import { HexString } from '@gear-js/api';

const VARA_NODE_ADDRESS = import.meta.env.VITE_VARA_NODE_ADDRESS as string;
const ETH_NODE_ADDRESS = import.meta.env.VITE_ETH_NODE_ADDRESS as string;
const ETH_CHAIN_ID = Number(import.meta.env.VITE_ETH_CHAIN_ID as string);

const VFT_MANAGER_CONTRACT_ADDRESS = import.meta.env.VITE_VFT_MANAGER_CONTRACT_ADDRESS as HexString;

// TODO: read from vft manager once it's state is implemented
const WRAPPED_VARA_CONTRACT_ADDRESS = import.meta.env.VITE_WRAPPED_VARA_CONTRACT_ADDRESS as HexString;
const WRAPPED_ETH_CONTRACT_ADDRESS = import.meta.env.VITE_WRAPPED_ETH_CONTRACT_ADDRESS as HexString;
const WRAPPED_USDC_CONTRACT_ADDRESS = import.meta.env.VITE_WRAPPED_USDC_CONTRACT_ADDRESS as HexString;
const WRAPPED_USDT_CONTRACT_ADDRESS = import.meta.env.VITE_WRAPPED_USDT_CONTRACT_ADDRESS as HexString;

// cuz eth addresses can be copied with mixed-case letters
const getLowerCaseAddress = (key: string) =>
  (import.meta.env[`VITE_${key}`] as string).toLocaleLowerCase() as HexString;

const ETH_WRAPPED_ETH_CONTRACT_ADDRESS = getLowerCaseAddress('ETH_WRAPPED_ETH_CONTRACT_ADDRESS');
const ETH_WRAPPED_VARA_CONTRACT_ADDRESS = getLowerCaseAddress('ETH_WRAPPED_VARA_CONTRACT_ADDRESS');
const USDC_CONTRACT_ADDRESS = getLowerCaseAddress('USDC_CONTRACT_ADDRESS');
const USDT_CONTRACT_ADDRESS = getLowerCaseAddress('USDT_CONTRACT_ADDRESS');

const GTM_ID = import.meta.env.VITE_GTM_ID as string | undefined;

export {
  VARA_NODE_ADDRESS,
  ETH_NODE_ADDRESS,
  ETH_CHAIN_ID,
  VFT_MANAGER_CONTRACT_ADDRESS,
  WRAPPED_VARA_CONTRACT_ADDRESS,
  WRAPPED_ETH_CONTRACT_ADDRESS,
  WRAPPED_USDC_CONTRACT_ADDRESS,
  WRAPPED_USDT_CONTRACT_ADDRESS,
  ETH_WRAPPED_ETH_CONTRACT_ADDRESS,
  ETH_WRAPPED_VARA_CONTRACT_ADDRESS,
  USDC_CONTRACT_ADDRESS,
  USDT_CONTRACT_ADDRESS,
  GTM_ID,
};
