import { HexString } from '@gear-js/api';

const BRIDGING_PAYMENT_CONTRACT_ADDRESS = import.meta.env.VITE_BRIDGING_PAYMENT_CONTRACT_ADDRESS as HexString;

// TODO: read from vft manager once it's state is implemented
const WRAPPED_VARA_CONTRACT_ADDRESS = import.meta.env.VITE_WRAPPED_VARA_CONTRACT_ADDRESS as HexString;

// TODO: can be read from vara bridging payment?
const ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS = import.meta.env.VITE_ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS as HexString;

export { BRIDGING_PAYMENT_CONTRACT_ADDRESS, WRAPPED_VARA_CONTRACT_ADDRESS, ETH_BRIDGING_PAYMENT_CONTRACT_ADDRESS };
