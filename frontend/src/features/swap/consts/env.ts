import { HexString } from '@gear-js/api';

// TODO: can be read from vara bridging payment?
const ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS = import.meta.env.VITE_ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS as HexString;

export { ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS };
