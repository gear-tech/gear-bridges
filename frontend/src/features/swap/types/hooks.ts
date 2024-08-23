import { HexString } from '@gear-js/api';

import { FormattedValues } from './form';
import { Contract } from './spec';

type BalanceValues = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

type Config = {
  fee: BalanceValues;
  minValue: bigint | undefined;
  ftAddress: HexString | undefined;
  isLoading: boolean;
};

type UseBalance = (config: Config) => BalanceValues & {
  decimals: number | undefined;
  isLoading: boolean;
};

type UseConfig = (contract: Contract) => Config;

type UseHandleSubmit = (
  contract: Contract,
  config: Config,
) => {
  onSubmit: (values: FormattedValues, reset: () => void) => void;
  isSubmitting: boolean;
};

export type { Config, UseBalance, UseConfig, UseHandleSubmit };
