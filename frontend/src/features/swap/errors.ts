import { ERROR_MESSAGE } from './consts';

class InsufficientAccountBalanceError extends Error {
  balanceValue: bigint;
  minValue: bigint;

  constructor(balanceValue: bigint, minValue: bigint) {
    super(ERROR_MESSAGE.NO_ACCOUNT_BALANCE);
    this.balanceValue = balanceValue;
    this.minValue = minValue;
  }
}

export { InsufficientAccountBalanceError };
