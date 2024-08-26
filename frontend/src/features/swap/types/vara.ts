import { HexString } from '@gear-js/api';

type ConfigState = {
  min_amount: number;
  min_validators_required: 3;
  gas_for_migration: number;
  ft_token_id?: HexString;
};

export type { ConfigState };
