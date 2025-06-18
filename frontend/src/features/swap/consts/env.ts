import { HexString } from '@gear-js/api';

const CONTRACT_ADDRESS = {
  VFT_MANAGER: import.meta.env.VITE_VFT_MANAGER_CONTRACT_ADDRESS as HexString,
  BRIDGING_PAYMENT: import.meta.env.VITE_BRIDGING_PAYMENT_CONTRACT_ADDRESS as HexString,

  ETH_BRIDGING_PAYMENT: import.meta.env.VITE_ETH_BRIDGING_PAYMENT_CONTRACT as HexString,
  ERC20_MANAGER: import.meta.env.VITE_ERC20_MANAGER_CONTRACT_ADDRESS as HexString,
} as const;

export { CONTRACT_ADDRESS };
