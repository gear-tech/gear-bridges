import { HexString } from '@gear-js/api';

type ConfigState = {
  Config: {
    minAmount: string;
    minValidatorsRequired: string;
    gasForMigration: string;
    fee: string;
    ftTokenId?: HexString;
  };
};

export type { ConfigState };
