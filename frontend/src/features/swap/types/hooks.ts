import { HexString } from '@gear-js/api';

import { FormattedValues } from './form';
import { Contract } from './spec';

type BalanceValues = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

type Config = {
  minValue: bigint | undefined;
  ftAddress: HexString | undefined;
  isLoading: boolean;
};

type FeeCalculatorResponse = {
  fee: string;
  mortality: number;
  timestamp: number;
  bytes: string;
  signature: HexString;
};

type FeeCalculator = {
  fee: { value: bigint; formattedValue: string };
  mortality: number;
  timestamp: number;
  bytes: string;
  signature: HexString;
};

type UseBalance = (config: Config) => BalanceValues & {
  decimals: number | undefined;
  isLoading: boolean;
};

type UseConfig = (contract: Contract) => Config;

type UseHandleSubmit = (
  contract: Contract,
  config: Config,
  feeCalculator?: FeeCalculator,
) => {
  onSubmit: (values: FormattedValues, reset: () => void) => void;
  isSubmitting: boolean;
};

export type { Config, UseBalance, UseConfig, UseHandleSubmit, FeeCalculatorResponse, FeeCalculator };
