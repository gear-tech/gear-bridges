import { NETWORK_PRESET, NETWORK_TYPE } from './consts';

type NetworkType = (typeof NETWORK_TYPE)[keyof typeof NETWORK_TYPE];

type NetworkPreset = (typeof NETWORK_PRESET)[keyof typeof NETWORK_PRESET];

export type { NetworkType, NetworkPreset };
