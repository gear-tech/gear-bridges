import { HexString } from '@gear-js/api';

type BalanceValues = {
  value: bigint | undefined;
  formattedValue: string | undefined;
};

type UseAccountBalance = () => {
  data: bigint | undefined;
  isLoading: boolean;
};

type UseFTBalance = (ftAddress: HexString | undefined) => {
  data: bigint | undefined;
  isLoading: boolean;
};

type UseFee = () => {
  bridgingFee: BalanceValues;
  isLoading: boolean;
  vftManagerFee?: BalanceValues;
};

export type { UseAccountBalance, UseFTBalance, UseFee };
