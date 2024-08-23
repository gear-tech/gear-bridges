import { HexString, ProgramMetadata } from '@gear-js/api';

import { NETWORK_NAME } from '@/consts';

import { TOKEN_TYPE } from '../consts';

type NetworkName = (typeof NETWORK_NAME)[keyof typeof NETWORK_NAME];
type TokenType = (typeof TOKEN_TYPE)[keyof typeof TOKEN_TYPE];

type Bridge = {
  network: NetworkName;
  address: HexString;
  symbol: string;
  tokenType: TokenType;
  decimals: number;
};

type Contract = {
  address: HexString;
  metadata?: ProgramMetadata | undefined;
};

export type { NetworkName, TokenType, Bridge, Contract };
